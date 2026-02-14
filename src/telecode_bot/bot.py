from __future__ import annotations

import asyncio
import shlex
from dataclasses import dataclass
from pathlib import Path
from time import monotonic
from typing import Any

from telecode_bot.config import BotConfig
from telecode_bot.db import ThreadStore
from telecode_bot.engines import build_engine_command, parse_engine_stream_line
from telecode_bot.telegram_api import TelegramAPI, TelegramApiError, TelegramMessageRef
from telecode_bot.types import (
    IncomingTask,
    MessageContext,
    ParsedCommand,
    RunContext,
    RunResult,
    ThreadState,
)
from telecode_bot.utils import (
    RollingBuffer,
    build_thread_key,
    compact_whitespace,
    parse_command,
    truncate_for_telegram,
    utc_now_iso,
)


@dataclass
class ThreadWorker:
    thread_key: str
    bot: TelegramBridgeBot

    def __post_init__(self) -> None:
        self.queue: asyncio.Queue[IncomingTask] = asyncio.Queue()
        self.running = False
        self.cancel_requested = False
        self.current_process: asyncio.subprocess.Process | None = None
        self._loop_task = asyncio.create_task(self._run_loop())

    async def enqueue(self, task: IncomingTask) -> None:
        await self.queue.put(task)

    async def cancel_current(self) -> bool:
        self.cancel_requested = True
        if self.current_process is None or self.current_process.returncode is not None:
            return False
        await terminate_process(self.current_process)
        return True

    async def _run_loop(self) -> None:
        while True:
            task = await self.queue.get()
            try:
                self.running = True
                self.cancel_requested = False
                await self.bot.process_task(self, task)
            except Exception as exc:
                await self.bot.reply_text(
                    task.message.chat_id,
                    task.message.thread_id,
                    f"❌ Internal error: {exc}",
                )
            finally:
                self.running = False
                self.current_process = None
                self.queue.task_done()


async def terminate_process(process: asyncio.subprocess.Process) -> None:
    if process.returncode is not None:
        return
    process.terminate()
    try:
        await asyncio.wait_for(process.wait(), timeout=3.0)
    except TimeoutError:
        process.kill()
        await process.wait()


class TelegramBridgeBot:
    def __init__(self, config: BotConfig) -> None:
        self.config = config
        self.telegram = TelegramAPI(config.telegram_bot_token)
        self.store = ThreadStore(
            db_path=config.db_path,
            workspace_root=config.workspace_root,
            default_engine=config.default_engine,
            default_tool_mode=config.default_tool_mode,
        )
        self._offset: int | None = None
        self._workers: dict[str, ThreadWorker] = {}

    async def run_forever(self) -> None:
        while True:
            try:
                updates = await asyncio.to_thread(
                    self.telegram.get_updates,
                    self._offset,
                    self.config.poll_timeout_seconds,
                )
                for update in updates:
                    self._offset = int(update["update_id"]) + 1
                    await self._handle_update(update)
            except TelegramApiError as exc:
                print(f"[{utc_now_iso()}] Telegram API error: {exc}")
                await asyncio.sleep(2.0)
            except Exception as exc:
                print(f"[{utc_now_iso()}] Fatal polling error: {exc}")
                await asyncio.sleep(2.0)

    async def _handle_update(self, update: dict[str, Any]) -> None:
        message = update.get("message")
        if not isinstance(message, dict):
            return

        text = message.get("text")
        if not isinstance(text, str):
            return

        user = message.get("from", {})
        user_id = int(user.get("id", 0))
        if user_id not in self.config.allowed_user_ids:
            return

        chat = message.get("chat", {})
        chat_id = int(chat.get("id", 0))
        thread_id = message.get("message_thread_id")
        parsed_thread_id = int(thread_id) if isinstance(thread_id, int) else None
        thread_key = build_thread_key(chat_id, parsed_thread_id)

        task = IncomingTask(
            message=MessageContext(
                chat_id=chat_id,
                user_id=user_id,
                text=text,
                message_id=int(message.get("message_id", 0)),
                thread_id=parsed_thread_id,
            ),
            command=parse_command(text),
        )

        worker = self._workers.get(thread_key)
        if worker is None:
            worker = ThreadWorker(thread_key=thread_key, bot=self)
            self._workers[thread_key] = worker

        if task.command is not None and task.command.name == "cancel":
            cancelled = await worker.cancel_current()
            if cancelled:
                await self.reply_text(
                    chat_id,
                    parsed_thread_id,
                    "🛑 Cancel requested. Stopping current run.",
                )
            else:
                await self.reply_text(chat_id, parsed_thread_id, "No active run to cancel.")
            return

        was_busy = worker.running
        await worker.enqueue(task)
        if was_busy:
            position = worker.queue.qsize()
            await self.reply_text(
                chat_id,
                parsed_thread_id,
                f"Thread is busy. Queued at position {position}. Use /cancel to stop current run.",
            )

    async def process_task(self, worker: ThreadWorker, task: IncomingTask) -> None:
        thread_state = self.store.get_or_create_thread(worker.thread_key)
        if task.command is not None:
            await self._handle_command(worker, task, thread_state, task.command)
            return
        await self._run_user_prompt(worker, task, thread_state, task.message.text)

    async def _handle_command(
        self,
        worker: ThreadWorker,
        task: IncomingTask,
        thread_state: ThreadState,
        command: ParsedCommand,
    ) -> None:
        if command.name == "new_thread":
            self.store.set_active_session(thread_state.thread_key, None)
            self.store.set_compact_summary(thread_state.thread_key, None)
            await self.reply_text(
                task.message.chat_id,
                task.message.thread_id,
                "Started a fresh session for this thread.",
            )
            return

        if command.name == "resume":
            await self._handle_resume(task, thread_state, command.args)
            return

        if command.name == "clear":
            fresh_state = self.store.clear_thread_state(thread_state.thread_key)
            await self.reply_text(
                task.message.chat_id,
                task.message.thread_id,
                (
                    "Thread memory cleared. "
                    f"New workspace: {fresh_state.workspace_path}"
                ),
            )
            return

        if command.name == "compact":
            await self._handle_compact(worker, task, thread_state)
            return

        if command.name == "engine":
            await self._handle_engine_switch(task, thread_state, command.args)
            return

        if command.name == "toolmode":
            await self._handle_toolmode_switch(task, thread_state, command.args)
            return

        if command.name == "status":
            await self._handle_status(task, thread_state)
            return

        if command.name == "publish":
            await self._handle_publish(task, thread_state, command.args)
            return

        await self.reply_text(
            task.message.chat_id,
            task.message.thread_id,
            (
                "Unknown command. Supported: "
                "/new_thread /resume <id> /clear /compact /cancel /engine <claude|codex> "
                "/toolmode <safe|full> /status /publish <repo-name> [private|public]"
            ),
        )

    async def _handle_resume(
        self,
        task: IncomingTask,
        thread_state: ThreadState,
        args: str,
    ) -> None:
        if args:
            session_id = args.strip()
            self.store.set_active_session(thread_state.thread_key, session_id)
            self.store.add_session_history(
                thread_state.thread_key,
                thread_state.active_engine,
                session_id,
            )
            await self.reply_text(
                task.message.chat_id,
                task.message.thread_id,
                f"Active session set to `{session_id}`.",
            )
            return

        sessions = self.store.recent_sessions(thread_state.thread_key)
        if not sessions:
            await self.reply_text(
                task.message.chat_id,
                task.message.thread_id,
                "No stored session IDs for this thread. Use /resume <session_id>.",
            )
            return

        formatted = "\n".join(f"- `{session_id}`" for session_id in sessions)
        await self.reply_text(
            task.message.chat_id,
            task.message.thread_id,
            f"Recent sessions:\n{formatted}",
        )

    async def _handle_engine_switch(
        self,
        task: IncomingTask,
        thread_state: ThreadState,
        args: str,
    ) -> None:
        selected = args.strip().lower()
        if selected not in {"claude", "codex"}:
            await self.reply_text(
                task.message.chat_id,
                task.message.thread_id,
                "Usage: /engine <claude|codex>",
            )
            return

        self.store.set_active_engine(thread_state.thread_key, selected)
        await self.reply_text(
            task.message.chat_id,
            task.message.thread_id,
            f"Engine set to {selected}.",
        )

    async def _handle_toolmode_switch(
        self,
        task: IncomingTask,
        thread_state: ThreadState,
        args: str,
    ) -> None:
        selected = args.strip().lower()
        if selected not in {"safe", "full"}:
            await self.reply_text(
                task.message.chat_id,
                task.message.thread_id,
                "Usage: /toolmode <safe|full>",
            )
            return

        self.store.set_tool_mode(thread_state.thread_key, selected)
        await self.reply_text(
            task.message.chat_id,
            task.message.thread_id,
            f"Tool mode set to {selected}.",
        )

    async def _handle_status(self, task: IncomingTask, thread_state: ThreadState) -> None:
        recent_sessions = self.store.recent_sessions(thread_state.thread_key)
        latest = recent_sessions[0] if recent_sessions else "none"
        status = (
            f"Thread: {thread_state.thread_key}\n"
            f"Engine: {thread_state.active_engine}\n"
            f"Workspace: {thread_state.workspace_path}\n"
            f"Active session: {thread_state.active_session_id or 'none'}\n"
            f"Compact summary: {'yes' if thread_state.compact_summary else 'no'}\n"
            f"Tool mode: {thread_state.tool_mode}\n"
            f"Latest stored session: {latest}"
        )
        await self.reply_text(task.message.chat_id, task.message.thread_id, status)

    async def _handle_compact(
        self,
        worker: ThreadWorker,
        task: IncomingTask,
        thread_state: ThreadState,
    ) -> None:
        status = await self.reply_text(
            task.message.chat_id,
            task.message.thread_id,
            "🟡 Compacting memory...",
        )
        run_ctx = self._create_run_context(thread_state, status.message_id)

        prompt = (
            "Create a strict JSON summary for session compaction.\n"
            "Return JSON object with keys: decisions, current_state, "
            "file_changes, next_steps, constraints.\n"
            "Keep it concise and factual."
        )

        result = await self._execute_engine_capture(
            worker=worker,
            thread_state=thread_state,
            run_context=run_ctx,
            prompt=prompt,
            force_session_id=thread_state.active_session_id,
            seed_summary=None,
        )
        self.store.finish_run(run_ctx.run_id, result.status)

        if result.status != "succeeded":
            await self.edit_text(
                task.message.chat_id,
                status.message_id,
                f"❌ Compact failed ({result.status}).\n\n{result.output_tail}",
            )
            return

        summary = result.output_tail.strip()
        if not summary:
            await self.edit_text(
                task.message.chat_id,
                status.message_id,
                "❌ Compact failed: no summary produced.",
            )
            return

        self.store.set_compact_summary(thread_state.thread_key, summary)
        self.store.set_active_session(thread_state.thread_key, None)

        seed_run_ctx = self._create_run_context(thread_state, status.message_id)
        seed_result = await self._execute_engine_capture(
            worker=worker,
            thread_state=thread_state,
            run_context=seed_run_ctx,
            prompt="Memory loaded. Reply exactly MEMORY_READY.",
            force_session_id=None,
            seed_summary=summary,
        )
        self.store.finish_run(seed_run_ctx.run_id, seed_result.status)
        if seed_result.session_id:
            self.store.set_active_session(thread_state.thread_key, seed_result.session_id)
            self.store.add_session_history(
                thread_state.thread_key,
                thread_state.active_engine,
                seed_result.session_id,
            )

        await self.edit_text(
            task.message.chat_id,
            status.message_id,
            (
                "✅ Compact complete.\n"
                f"Summary stored ({len(summary)} chars).\n"
                f"Seed session: {seed_result.session_id or 'created on next message'}"
            ),
        )

    async def _run_user_prompt(
        self,
        worker: ThreadWorker,
        task: IncomingTask,
        thread_state: ThreadState,
        prompt: str,
    ) -> None:
        status_ref = await self.reply_text(
            task.message.chat_id,
            task.message.thread_id,
            f"🟡 Running {thread_state.active_engine}...",
        )
        run_context = self._create_run_context(thread_state, status_ref.message_id)

        seed_summary = None
        if thread_state.active_session_id is None and thread_state.compact_summary:
            seed_summary = thread_state.compact_summary

        result = await self._execute_engine_stream(
            worker=worker,
            thread_state=thread_state,
            run_context=run_context,
            prompt=prompt,
            chat_id=task.message.chat_id,
            status_message_id=status_ref.message_id,
            force_session_id=thread_state.active_session_id,
            seed_summary=seed_summary,
        )

        self.store.finish_run(run_context.run_id, result.status)
        if result.session_id:
            self.store.set_active_session(thread_state.thread_key, result.session_id)
            self.store.add_session_history(
                thread_state.thread_key,
                thread_state.active_engine,
                result.session_id,
            )

        icon = "✅" if result.status == "succeeded" else "❌"
        if result.status == "canceled":
            icon = "🛑"

        final_text = (
            f"{icon} {result.status.upper()}\n"
            f"Engine: {thread_state.active_engine}\n"
            f"Run: {run_context.run_id}\n"
            f"Session: {result.session_id or thread_state.active_session_id or 'unknown'}\n\n"
            f"{result.output_tail}"
        )
        await self.edit_text(task.message.chat_id, status_ref.message_id, final_text)

    def _create_run_context(self, thread_state: ThreadState, status_message_id: int) -> RunContext:
        log_path = self._build_log_path(thread_state.thread_key)
        run_id = self.store.create_run(
            thread_key=thread_state.thread_key,
            engine=thread_state.active_engine,
            last_telegram_message_id=status_message_id,
            log_path=log_path,
        )
        return RunContext(run_id=run_id, status_message_id=status_message_id, log_path=log_path)

    def _build_log_path(self, thread_key: str) -> Path:
        thread_dir = self.config.logs_root / thread_key.replace(":", "_")
        thread_dir.mkdir(parents=True, exist_ok=True)
        return thread_dir / f"run-{utc_now_iso().replace(':', '').replace('+00:00', 'Z')}.log"

    async def _execute_engine_stream(
        self,
        worker: ThreadWorker,
        thread_state: ThreadState,
        run_context: RunContext,
        prompt: str,
        chat_id: int,
        status_message_id: int,
        force_session_id: str | None,
        seed_summary: str | None,
    ) -> RunResult:
        command, effective_prompt = build_engine_command(
            engine=thread_state.active_engine,
            config=self.config,
            prompt=prompt,
            session_id=force_session_id,
            tool_mode=thread_state.tool_mode,
            seed_summary=seed_summary,
        )

        process = await asyncio.create_subprocess_exec(
            *command,
            cwd=str(thread_state.workspace_path),
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
        )
        if process.stdout is None:
            raise RuntimeError("Failed to capture engine stdout")
        stdout = process.stdout
        worker.current_process = process

        rolling = RollingBuffer(max_chars=self.config.max_output_chars)
        session_id = force_session_id
        last_edit = monotonic()
        last_payload = ""
        timed_out = False
        start_time = monotonic()

        run_header = (
            f"🟡 Running {thread_state.active_engine} | run {run_context.run_id}\n"
            f"Workspace: {thread_state.workspace_path}\n"
            f"Prompt: {compact_whitespace(effective_prompt)[:120]}\n\n"
        )

        with run_context.log_path.open("a", encoding="utf-8") as log_file:
            log_file.write("$ " + " ".join(shlex.quote(arg) for arg in command) + "\n")

            while True:
                if monotonic() - start_time > self.config.max_runtime_seconds:
                    timed_out = True
                    await terminate_process(process)
                    break

                try:
                    line = await asyncio.wait_for(stdout.readline(), timeout=0.5)
                except TimeoutError:
                    line = b""

                if line:
                    decoded = line.decode("utf-8", errors="replace")
                    log_file.write(decoded)
                    chunk, parsed_session_id = parse_engine_stream_line(
                        thread_state.active_engine,
                        decoded,
                    )
                    if parsed_session_id:
                        session_id = parsed_session_id
                    if chunk:
                        rolling.append(chunk)

                if monotonic() - last_edit >= self.config.stream_edit_interval_seconds:
                    payload = run_header + (rolling.value.strip() or "(waiting for output)")
                    payload = truncate_for_telegram(payload)
                    if payload != last_payload:
                        await self.edit_text(chat_id, status_message_id, payload)
                        last_payload = payload
                    last_edit = monotonic()

                if worker.cancel_requested and process.returncode is None:
                    await terminate_process(process)

                if process.returncode is None:
                    try:
                        await asyncio.wait_for(process.wait(), timeout=0.01)
                    except TimeoutError:
                        continue
                break

            exit_code = await process.wait()
            if timed_out:
                status = "failed"
                rolling.append("\n[timeout] Run exceeded configured max runtime.\n")
            elif worker.cancel_requested:
                status = "canceled"
                rolling.append("\n[canceled] Run canceled by user.\n")
            elif exit_code == 0:
                status = "succeeded"
            else:
                status = "failed"
                rolling.append(f"\n[exit-code] {exit_code}\n")

        return RunResult(
            status=status,
            output_tail=rolling.value.strip() or "(no streamed output)",
            session_id=session_id,
            exit_code=exit_code,
        )

    async def _execute_engine_capture(
        self,
        worker: ThreadWorker,
        thread_state: ThreadState,
        run_context: RunContext,
        prompt: str,
        force_session_id: str | None,
        seed_summary: str | None,
    ) -> RunResult:
        command, _ = build_engine_command(
            engine=thread_state.active_engine,
            config=self.config,
            prompt=prompt,
            session_id=force_session_id,
            tool_mode=thread_state.tool_mode,
            seed_summary=seed_summary,
        )

        process = await asyncio.create_subprocess_exec(
            *command,
            cwd=str(thread_state.workspace_path),
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
        )
        if process.stdout is None:
            raise RuntimeError("Failed to capture engine stdout")
        stdout = process.stdout
        worker.current_process = process

        rolling = RollingBuffer(max_chars=12000)
        session_id = force_session_id
        timed_out = False
        start_time = monotonic()

        with run_context.log_path.open("a", encoding="utf-8") as log_file:
            log_file.write("$ " + " ".join(shlex.quote(arg) for arg in command) + "\n")
            while True:
                if monotonic() - start_time > self.config.max_runtime_seconds:
                    timed_out = True
                    await terminate_process(process)
                    break

                try:
                    line = await asyncio.wait_for(stdout.readline(), timeout=0.5)
                except TimeoutError:
                    line = b""

                if line:
                    decoded = line.decode("utf-8", errors="replace")
                    log_file.write(decoded)
                    chunk, parsed_session_id = parse_engine_stream_line(
                        thread_state.active_engine,
                        decoded,
                    )
                    if parsed_session_id:
                        session_id = parsed_session_id
                    if chunk:
                        rolling.append(chunk)

                if worker.cancel_requested and process.returncode is None:
                    await terminate_process(process)

                if process.returncode is None:
                    try:
                        await asyncio.wait_for(process.wait(), timeout=0.01)
                    except TimeoutError:
                        continue
                break

        exit_code = await process.wait()
        if timed_out:
            status = "failed"
            rolling.append("\n[timeout] Run exceeded configured max runtime.\n")
        elif worker.cancel_requested:
            status = "canceled"
            rolling.append("\n[canceled] Run canceled by user.\n")
        elif exit_code == 0:
            status = "succeeded"
        else:
            status = "failed"
            rolling.append(f"\n[exit-code] {exit_code}\n")

        return RunResult(
            status=status,
            output_tail=rolling.value.strip() or "(no streamed output)",
            session_id=session_id,
            exit_code=exit_code,
        )

    async def _handle_publish(
        self,
        task: IncomingTask,
        thread_state: ThreadState,
        args: str,
    ) -> None:
        chunks = args.split()
        repo_name = chunks[0] if chunks else f"telecode-{task.message.chat_id}"
        visibility = "private"
        if len(chunks) > 1 and chunks[1].lower() in {"private", "public"}:
            visibility = chunks[1].lower()

        status_ref = await self.reply_text(
            task.message.chat_id,
            task.message.thread_id,
            f"🟡 Publishing workspace `{thread_state.workspace_path}` to GitHub...",
        )

        workspace = thread_state.workspace_path
        workspace.mkdir(parents=True, exist_ok=True)

        commands = [
            ["git", "rev-parse", "--is-inside-work-tree"],
            ["git", "init", "-b", "main"],
            ["git", "add", "-A"],
            ["git", "diff", "--cached", "--quiet"],
            ["git", "commit", "-m", "[FEAT] telegram-bot publish snapshot"],
            ["git", "remote", "get-url", "origin"],
        ]

        init_result = await run_command(commands[0], cwd=workspace)
        if init_result["exit_code"] != 0:
            await run_command(commands[1], cwd=workspace)

        await run_command(commands[2], cwd=workspace)
        diff_result = await run_command(commands[3], cwd=workspace)
        if diff_result["exit_code"] == 1:
            await run_command(commands[4], cwd=workspace)

        remote_result = await run_command(commands[5], cwd=workspace)
        if remote_result["exit_code"] != 0:
            create_result = await run_command(
                [
                    "gh",
                    "repo",
                    "create",
                    repo_name,
                    f"--{visibility}",
                    "--source",
                    ".",
                    "--remote",
                    "origin",
                    "--push",
                ],
                cwd=workspace,
            )
            if create_result["exit_code"] != 0:
                await self.edit_text(
                    task.message.chat_id,
                    status_ref.message_id,
                    "❌ Publish failed.\n\n" + truncate_for_telegram(create_result["output"]),
                )
                return
        else:
            push_result = await run_command(
                ["git", "push", "-u", "origin", "HEAD"],
                cwd=workspace,
            )
            if push_result["exit_code"] != 0:
                await self.edit_text(
                    task.message.chat_id,
                    status_ref.message_id,
                    "❌ Push failed.\n\n" + truncate_for_telegram(push_result["output"]),
                )
                return

        url_result = await run_command(
            ["gh", "repo", "view", repo_name, "--json", "url", "--jq", ".url"],
            cwd=workspace,
        )
        repo_url = (
            url_result["output"].strip()
            if url_result["exit_code"] == 0
            else "(url unavailable)"
        )

        await self.edit_text(
            task.message.chat_id,
            status_ref.message_id,
            f"✅ Published to GitHub ({visibility}).\nRepo: {repo_url}",
        )

    async def reply_text(
        self,
        chat_id: int,
        thread_id: int | None,
        text: str,
    ) -> TelegramMessageRef:
        payload = truncate_for_telegram(text)
        return await asyncio.to_thread(self.telegram.send_message, chat_id, payload, thread_id)

    async def edit_text(self, chat_id: int, message_id: int, text: str) -> None:
        payload = truncate_for_telegram(text)
        await asyncio.to_thread(self.telegram.edit_message_text, chat_id, message_id, payload)


async def run_command(command: list[str], cwd: Path) -> dict[str, Any]:
    process = await asyncio.create_subprocess_exec(
        *command,
        cwd=str(cwd),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.STDOUT,
    )
    stdout, _ = await process.communicate()
    output = stdout.decode("utf-8", errors="replace").strip()
    return {
        "exit_code": process.returncode,
        "output": output,
        "command": " ".join(shlex.quote(part) for part in command),
    }
