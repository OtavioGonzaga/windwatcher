/// One OS signal source per process, shared by the serve and worker roles.
pub(crate) struct ShutdownSignal {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
}

impl ShutdownSignal {
    pub(crate) fn register() -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            Ok(Self {
                interrupt: signal(SignalKind::interrupt())?,
                terminate: signal(SignalKind::terminate())?,
            })
        }
        #[cfg(not(unix))]
        {
            Ok(Self {})
        }
    }

    pub(crate) async fn wait(mut self) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            let received = tokio::select! {
                value = self.interrupt.recv() => value,
                value = self.terminate.recv() => value,
            };
            received.ok_or_else(|| std::io::Error::other("shutdown signal stream closed"))
        }
        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c().await
        }
    }
}
