//! The bridge between the UI thread and `datadrop-core`: commands run on a tokio
//! runtime, and the UI is repainted when one finishes.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use datadrop_core::{CancellationToken, Client, Command, ConfigDir, Output, Result};

/// A command running in the background. Poll it each frame with [`poll`].
pub struct Task {
    slot: Arc<Mutex<Option<Result<Output>>>>,
    cancel: CancellationToken,
}

impl Task {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn is_cancelling(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

/// Take a finished task's result, clearing it. `None` while it runs (or if there's
/// no task).
pub fn poll(task: &mut Option<Task>) -> Option<Result<Output>> {
    let result = task.as_ref()?.slot.lock().expect("task slot").take();
    if result.is_some() {
        *task = None;
    }
    result
}

pub struct Backend {
    rt: tokio::runtime::Runtime,
    pub client: Arc<Client>,
    pub config: ConfigDir,
    ctx: egui::Context,
}

impl Backend {
    pub fn new(ctx: egui::Context, config: ConfigDir) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .thread_name("datadrop")
            .build()
            .expect("tokio runtime");
        Self {
            rt,
            client: Arc::new(Client::new(&config)),
            config,
            ctx,
        }
    }

    /// Run a command in the background.
    pub fn run(&self, cmd: Command) -> Task {
        let slot = Arc::new(Mutex::new(None));
        let cancel = CancellationToken::new();
        let (client, ctx, out, stop) = (
            self.client.clone(),
            self.ctx.clone(),
            slot.clone(),
            cancel.clone(),
        );
        self.rt.spawn(async move {
            let result = client.execute_cancellable(cmd, &stop).await;
            *out.lock().expect("task slot") = Some(result);
            ctx.request_repaint();
        });
        Task { slot, cancel }
    }

    /// Run a command and forget it (closing a tab's session, say).
    pub fn fire(&self, cmd: Command) {
        let client = self.client.clone();
        self.rt.spawn(async move {
            let _ = client.execute(cmd).await;
        });
    }

    /// Close every connection before the app exits.
    pub fn shutdown(&self) {
        let client = self.client.clone();
        self.rt.block_on(async move {
            let _ = tokio::time::timeout(Duration::from_secs(3), client.shutdown()).await;
        });
    }
}
