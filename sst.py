from __future__ import annotations

import argparse
import atexit
import json
import os
import subprocess
import sys
import tempfile
import threading
from pathlib import Path
from typing import ClassVar, Final

from moonshine_voice import get_model_for_language
from moonshine_voice.moonshine_api import ModelArch
from moonshine_voice.transcriber import Transcriber
from moonshine_voice.utils import get_model_path, load_wav_file

Text = str
AudioInput = bytes | bytearray | memoryview | str | Path

_PROJECT_ROOT: Final[Path] = Path(__file__).resolve().parent
_DEFAULT_CACHE_DIR: Final[Path] = (
    _PROJECT_ROOT / ".cache" / "moonshine_voice"
)

_ARCH_BY_NAME: Final[dict[str, ModelArch]] = {
    "tiny": ModelArch.TINY,
    "base": ModelArch.BASE,
    "tiny-streaming": ModelArch.TINY_STREAMING,
    "base-streaming": ModelArch.BASE_STREAMING,
    "small-streaming": ModelArch.SMALL_STREAMING,
    "medium-streaming": ModelArch.MEDIUM_STREAMING,
}


class SST:
    """Process-wide singleton STT wrapper for Moonshine Voice.

    Typical Telegram bot usage:

    ```python
    from sst import SST

    stt = SST(language="es", arch="base")
    text = stt.transcript(audio_bytes)
    ```
    """

    _instance: ClassVar[SST | None] = None
    _instance_lock: ClassVar[threading.Lock] = (
        threading.Lock()
    )
    language: str
    arch: ModelArch
    cache_dir: Path
    model_path: str
    model_arch: ModelArch
    _transcriber: Transcriber
    _transcribe_lock: threading.Lock
    _closed: bool
    _initialized: bool

    def __new__(
        cls, *args: object, **kwargs: object
    ) -> SST:
        if cls._instance is None:
            with cls._instance_lock:
                if cls._instance is None:
                    cls._instance = super().__new__(cls)
        return cls._instance

    def __init__(
        self,
        language: str = "en",
        arch: str | ModelArch = "tiny",
        cache_dir: str | Path | None = None,
    ) -> None:
        parsed_arch = _parse_arch(arch)
        normalized_language = language.strip().lower()
        resolved_cache_dir = (
            Path(cache_dir).expanduser().resolve()
            if cache_dir
            else _DEFAULT_CACHE_DIR.resolve()
        )

        if getattr(self, "_initialized", False):
            if (
                self.language != normalized_language
                or self.arch != parsed_arch
                or self.cache_dir != resolved_cache_dir
            ):
                raise RuntimeError(
                    "SST singleton already initialized"
                    " with different settings. "
                    "Create it once with your desired"
                    " language/arch/cache_dir."
                )
            return

        self.language = normalized_language
        self.arch = parsed_arch
        self.cache_dir = resolved_cache_dir

        _configure_cache(self.cache_dir)
        self.model_path, self.model_arch = (
            _resolve_model(self.language, self.arch)
        )

        self._transcriber = Transcriber(
            model_path=self.model_path,
            model_arch=self.model_arch,
        )
        self._transcriber.__enter__()
        self._transcribe_lock = threading.Lock()
        self._closed = False
        self._initialized = True
        atexit.register(self.close)

    def transcript(self, audio: AudioInput) -> Text:
        """Transcribe one audio payload to text.

        `audio` accepts:
        - `bytes`/`bytearray`/`memoryview`
        - `str`/`Path` pointing to an audio file
        """
        if self._closed:
            raise RuntimeError("SST instance is closed.")

        with _TempFiles() as temp_files:
            source_path = temp_files.materialize(audio)
            wav_path = temp_files.normalize_to_wav(
                source_path
            )
            audio_data, sample_rate = load_wav_file(
                wav_path
            )

            with self._transcribe_lock:
                transcript = (
                    self._transcriber
                    .transcribe_without_streaming(
                        audio_data, sample_rate
                    )
                )

        lines = [
            line.text.strip()
            for line in transcript.lines
            if line.text.strip()
        ]
        return "\n".join(lines)

    def close(self) -> None:
        if getattr(self, "_closed", True):
            return
        self._closed = True
        self._transcriber.__exit__(None, None, None)


class _TempFiles:
    def __init__(self) -> None:
        self._paths: list[Path] = []

    def __enter__(self) -> _TempFiles:
        return self

    def __exit__(
        self,
        exc_type: object,
        exc: object,
        tb: object,
    ) -> None:
        for path in self._paths:
            path.unlink(missing_ok=True)

    def _new_path(self, suffix: str) -> Path:
        with tempfile.NamedTemporaryFile(
            mode="wb", suffix=suffix, delete=False
        ) as file_obj:
            path = Path(file_obj.name)
        self._paths.append(path)
        return path

    def materialize(self, audio: AudioInput) -> Path:
        if isinstance(audio, (str, Path)):
            path = Path(audio).expanduser().resolve()
            if not path.exists():
                raise FileNotFoundError(
                    f"Audio file not found: {path}"
                )
            return path

        if isinstance(
            audio, (bytes, bytearray, memoryview)
        ):
            path = self._new_path(".input")
            path.write_bytes(bytes(audio))
            return path

        raise TypeError(
            "audio must be bytes, bytearray,"
            " memoryview, str, or Path"
        )

    def normalize_to_wav(
        self, source_path: Path
    ) -> Path:
        output_path = self._new_path(".wav")
        command = [
            "ffmpeg",
            "-y",
            "-i",
            str(source_path),
            "-vn",
            "-sn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-f",
            "wav",
            str(output_path),
        ]
        result = subprocess.run(
            command,
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode != 0:
            tail = "\n".join(
                result.stderr.splitlines()[-20:]
            )
            raise RuntimeError(
                "ffmpeg audio normalization"
                f" failed:\n{tail}"
            )
        return output_path


def get_sst(
    language: str = "en",
    arch: str | ModelArch = "tiny",
    cache_dir: str | Path | None = None,
) -> SST:
    """Factory: always returns the singleton."""
    return SST(
        language=language,
        arch=arch,
        cache_dir=cache_dir,
    )


def _parse_arch(value: str | ModelArch) -> ModelArch:
    if isinstance(value, ModelArch):
        return value

    lowered = value.strip().lower()
    if lowered in _ARCH_BY_NAME:
        return _ARCH_BY_NAME[lowered]

    if lowered.isdigit():
        try:
            return ModelArch(int(lowered))
        except ValueError as exc:
            raise ValueError(
                f"Invalid numeric model arch"
                f" '{value}'."
            ) from exc

    valid = ", ".join(sorted(_ARCH_BY_NAME.keys()))
    raise ValueError(
        f"Invalid model arch '{value}'."
        f" Valid values: {valid}, or numeric enum."
    )


def _configure_cache(cache_dir: Path) -> None:
    cache_dir.mkdir(parents=True, exist_ok=True)
    os.environ["MOONSHINE_VOICE_CACHE"] = str(
        cache_dir
    )


def _resolve_model(
    language: str, arch: ModelArch
) -> tuple[str, ModelArch]:
    if language == "en" and arch == ModelArch.TINY:
        path = str(get_model_path("tiny-en"))
        return path, ModelArch.TINY

    model_path, model_arch = get_model_for_language(
        wanted_language=language,
        wanted_model_arch=arch,
    )
    return model_path, ModelArch(model_arch)


def _check_ffmpeg() -> None:
    """Verify ffmpeg is available."""
    try:
        subprocess.run(
            ["ffmpeg", "-version"],
            capture_output=True,
            check=True,
        )
    except (
        FileNotFoundError,
        subprocess.CalledProcessError,
    ) as exc:
        raise RuntimeError(
            "ffmpeg is required for audio"
            " normalization but was not"
            " found on PATH"
        ) from exc


def _server_loop(stt: SST) -> int:
    """Persistent server mode.

    Protocol:
      -> (stdin)  one file path per line
      <- (stdout) one JSON per line:
         {"text": "..."} or {"error": "..."}

    First line: {"status": "ready"}
    """
    sys.stdout.write(
        json.dumps({"status": "ready"}) + "\n"
    )
    sys.stdout.flush()

    while True:
        line = sys.stdin.readline()
        if not line:
            break  # EOF — parent closed stdin

        audio_path = line.strip()
        if not audio_path:
            continue

        try:
            text = stt.transcript(audio_path)
            response = json.dumps({"text": text})
        except Exception as e:
            response = json.dumps({"error": str(e)})

        sys.stdout.write(response + "\n")
        sys.stdout.flush()

    return 0


def _main() -> int:
    parser = argparse.ArgumentParser(
        description="Self-contained singleton STT"
    )
    parser.add_argument(
        "audio",
        nargs="?",
        help="Input audio path (omit for --server)",
    )
    parser.add_argument(
        "--server",
        action="store_true",
        help="Persistent server: stdin/stdout JSON",
    )
    parser.add_argument(
        "--language", default="en"
    )
    parser.add_argument("--arch", default="tiny")
    parser.add_argument(
        "--cache-dir",
        default=str(_DEFAULT_CACHE_DIR),
    )
    args = parser.parse_args()

    # Validate ffmpeg before loading model
    _check_ffmpeg()

    stt = get_sst(
        language=args.language,
        arch=args.arch,
        cache_dir=args.cache_dir,
    )

    if args.server:
        return _server_loop(stt)

    if not args.audio:
        parser.error(
            "audio path required unless --server"
        )

    print(stt.transcript(args.audio))
    return 0


if __name__ == "__main__":
    raise SystemExit(_main())
