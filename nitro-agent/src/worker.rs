use std::collections::HashMap;
use std::sync::Arc;

use tokio::process::Child;
use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};

use crate::bot::BotCore;
use crate::types::{IncomingTask, ThreadKey};

/// Per-thread worker managing sequential task execution and subprocess lifecycle.
#[allow(dead_code)]
pub struct ThreadWorker {
    pub thread_key: ThreadKey,
    tx: mpsc::UnboundedSender<IncomingTask>,
    pub running: Arc<Mutex<bool>>,
    pub cancel_requested: Arc<Mutex<bool>>,
    pub current_process: Arc<Mutex<Option<Child>>>,
}

impl ThreadWorker {
    pub fn spawn(thread_key: ThreadKey, bot: Arc<BotCore>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let running = Arc::new(Mutex::new(false));
        let cancel_requested = Arc::new(Mutex::new(false));
        let current_process: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));

        let worker = Self {
            thread_key: thread_key.clone(),
            tx,
            running: running.clone(),
            cancel_requested: cancel_requested.clone(),
            current_process: current_process.clone(),
        };

        tokio::spawn(worker_loop(
            thread_key,
            rx,
            bot,
            running,
            cancel_requested,
            current_process,
        ));

        worker
    }

    pub fn enqueue(&self, task: IncomingTask) -> Result<(), mpsc::error::SendError<IncomingTask>> {
        self.tx.send(task)
    }

    pub async fn cancel_current(&self) -> bool {
        *self.cancel_requested.lock().await = true;
        let mut proc_guard = self.current_process.lock().await;
        if let Some(ref mut child) = *proc_guard {
            if let Err(e) = child.kill().await {
                warn!("Failed to kill subprocess: {e}");
                return false;
            }
            return true;
        }
        false
    }

    pub async fn is_running(&self) -> bool {
        *self.running.lock().await
    }

    #[allow(dead_code)]
    pub fn queue_size(&self) -> usize {
        // UnboundedSender doesn't expose queue size; we track it externally if needed.
        0
    }
}

async fn worker_loop(
    thread_key: ThreadKey,
    mut rx: mpsc::UnboundedReceiver<IncomingTask>,
    bot: Arc<BotCore>,
    running: Arc<Mutex<bool>>,
    cancel_requested: Arc<Mutex<bool>>,
    current_process: Arc<Mutex<Option<Child>>>,
) {
    info!(thread_key = %thread_key, "Worker loop started");

    while let Some(task) = rx.recv().await {
        {
            *running.lock().await = true;
            *cancel_requested.lock().await = false;
        }

        if let Err(e) = bot
            .process_task(&thread_key, &task, &cancel_requested, &current_process)
            .await
        {
            warn!(
                thread_key = %thread_key,
                error = %e,
                "Task processing failed"
            );
            let _ = bot
                .send_text(
                    task.message.chat_id,
                    task.message.thread_id,
                    &format!("Internal error: {e}"),
                )
                .await;
        }

        {
            *running.lock().await = false;
            *current_process.lock().await = None;
        }
    }

    info!(thread_key = %thread_key, "Worker loop ended");
}

/// Registry of all active thread workers.
pub struct WorkerRegistry {
    workers: Mutex<HashMap<ThreadKey, Arc<ThreadWorker>>>,
    bot: Arc<BotCore>,
}

impl WorkerRegistry {
    pub fn new(bot: Arc<BotCore>) -> Self {
        Self {
            workers: Mutex::new(HashMap::new()),
            bot,
        }
    }

    pub async fn get_or_create(&self, thread_key: &ThreadKey) -> Arc<ThreadWorker> {
        let mut workers = self.workers.lock().await;
        if let Some(worker) = workers.get(thread_key) {
            return worker.clone();
        }

        let worker = Arc::new(ThreadWorker::spawn(thread_key.clone(), self.bot.clone()));
        workers.insert(thread_key.clone(), worker.clone());
        worker
    }
}
