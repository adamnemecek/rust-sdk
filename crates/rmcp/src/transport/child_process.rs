use process_wrap::tokio::{TokioChildWrapper, TokioCommandWrap};
use tokio::{
    io::AsyncRead,
    process::{ChildStdin, ChildStdout},
};

use super::{IntoTransport, Transport};
use crate::service::ServiceRole;

pub(crate) fn child_process(
    mut child: Box<dyn TokioChildWrapper>,
) -> std::io::Result<(Box<dyn TokioChildWrapper>, (ChildStdout, ChildStdin))> {
    let child_stdin = match child.inner_mut().stdin().take() {
        Some(stdin) => stdin,
        None => return Err(std::io::Error::other("std in was taken")),
    };
    let child_stdout = match child.inner_mut().stdout().take() {
        Some(stdout) => stdout,
        None => return Err(std::io::Error::other("std out was taken")),
    };
    Ok((child, (child_stdout, child_stdin)))
}

pub struct TokioChildProcess {
    child: ChildWithCleanup,
    child_stdin: ChildStdin,
    child_stdout: ChildStdout,
}

pub struct ChildWithCleanup {
    inner: Box<dyn TokioChildWrapper>,
}

impl Drop for ChildWithCleanup {
    fn drop(&mut self) {
        if let Err(e) = self.inner.start_kill() {
            tracing::warn!("Failed to kill child process: {e}");
        }
    }
}

// we hold the child process with stdout, for it's easier to implement AsyncRead
pin_project_lite::pin_project! {
    pub struct TokioChildProcessOut {
        child: ChildWithCleanup,
        #[pin]
        child_stdout: ChildStdout,
    }
}

impl TokioChildProcessOut {
    /// Get the process ID of the child process.
    pub fn id(&self) -> Option<u32> {
        self.child.inner.id()
    }
}

impl AsyncRead for TokioChildProcessOut {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.project().child_stdout.poll_read(cx, buf)
    }
}

impl TokioChildProcess {
    /// Create a new Tokio child process with the given command.
    ///
    /// # Manage the child process
    /// You can also check these issue and pr for more information on how to manage the child process:
    /// - [#156](https://github.com/modelcontextprotocol/rust-sdk/pull/156)
    /// - [#253](https://github.com/modelcontextprotocol/rust-sdk/issues/253)
    /// ```rust,ignore
    /// #[cfg(unix)]
    /// command_wrap.wrap(process_wrap::tokio::ProcessGroup::leader());
    /// #[cfg(windows)]
    /// command_wrap.wrap(process_wrap::tokio::JobObject);
    /// ```
    ///
    pub fn new(command: impl Into<TokioCommandWrap>) -> std::io::Result<Self> {
        let mut command_wrap = command.into();
        command_wrap
            .command_mut()
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());
        let (child, (child_stdout, child_stdin)) = child_process(command_wrap.spawn()?)?;
        Ok(Self {
            child: ChildWithCleanup { inner: child },
            child_stdin,
            child_stdout,
        })
    }

    /// Get the process ID of the child process.
    pub fn id(&self) -> Option<u32> {
        self.child.inner.id()
    }

    pub fn split(self) -> (TokioChildProcessOut, ChildStdin) {
        let Self {
            child,
            child_stdin,
            child_stdout,
        } = self;
        (
            TokioChildProcessOut {
                child,
                child_stdout,
            },
            child_stdin,
        )
    }
}

impl<R: ServiceRole> IntoTransport<R, std::io::Error, ()> for TokioChildProcess {
    fn into_transport(self) -> impl Transport<R, Error = std::io::Error> + 'static {
        IntoTransport::<R, std::io::Error, super::async_rw::TransportAdapterAsyncRW>::into_transport(
            self.split(),
        )
    }
}

pub trait ConfigureCommandExt {
    fn configure(self, f: impl FnOnce(&mut Self)) -> Self;
}

impl ConfigureCommandExt for tokio::process::Command {
    fn configure(mut self, f: impl FnOnce(&mut Self)) -> Self {
        f(&mut self);
        self
    }
}
