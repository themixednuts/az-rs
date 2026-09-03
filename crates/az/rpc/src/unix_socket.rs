use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use fs2::FileExt;

/// A filesystem Unix listener and the exclusive lease for its endpoint path.
///
/// All Azoth services that bind filesystem Unix sockets must retain this value,
/// or the UnixSocketLease returned by into_parts, for the listener lifetime.
/// The adjacent lock serializes stale recovery and the endpoint inode prevents
/// shutdown from removing a replacement socket.
pub struct OwnedUnixListener {
    listener: UnixListener,
    lease: UnixSocketLease,
}

impl OwnedUnixListener {
    /// Bind `path`, reclaiming it only when it is a stale socket with no live listener.
    pub fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let lock = acquire_endpoint_lock(path)?;
        let listener = bind_or_reclaim_stale(path)?;
        let metadata = std::fs::symlink_metadata(path)?;
        let lease = UnixSocketLease {
            path: path.to_path_buf(),
            identity: SocketIdentity::from_metadata(&metadata),
            _lock: lock,
        };

        Ok(Self { listener, lease })
    }

    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        self.listener.set_nonblocking(nonblocking)
    }

    /// Separate the listener from its endpoint lease for runtime adaptation.
    ///
    /// The caller must retain the lease until the listener has stopped and been
    /// dropped. Dropping the lease conditionally removes the owned socket path.
    #[must_use]
    pub fn into_parts(self) -> (UnixListener, UnixSocketLease) {
        (self.listener, self.lease)
    }
}

/// The exclusive ownership lease for a bound filesystem Unix socket path.
pub struct UnixSocketLease {
    path: PathBuf,
    identity: SocketIdentity,
    _lock: File,
}

impl Drop for UnixSocketLease {
    fn drop(&mut self) {
        let Ok(metadata) = std::fs::symlink_metadata(&self.path) else {
            return;
        };
        if metadata.file_type().is_socket()
            && SocketIdentity::from_metadata(&metadata) == self.identity
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SocketIdentity {
    device: u64,
    inode: u64,
}

impl SocketIdentity {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

fn acquire_endpoint_lock(path: &Path) -> io::Result<File> {
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(endpoint_lock_path(path))?;
    FileExt::try_lock_exclusive(&lock).map_err(|error| {
        if error.kind() == ErrorKind::WouldBlock {
            io::Error::new(
                ErrorKind::AddrInUse,
                "Unix socket endpoint is already owned",
            )
        } else {
            error
        }
    })?;
    Ok(lock)
}

fn endpoint_lock_path(path: &Path) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(".lock");
    PathBuf::from(name)
}

fn bind_or_reclaim_stale(path: &Path) -> io::Result<UnixListener> {
    match UnixListener::bind(path) {
        Ok(listener) => Ok(listener),
        Err(bind_error) if bind_error.kind() == ErrorKind::AddrInUse => {
            reclaim_stale_socket(path, bind_error)
        }
        Err(error) => Err(error),
    }
}

fn reclaim_stale_socket(path: &Path, bind_error: io::Error) -> io::Result<UnixListener> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return UnixListener::bind(path),
        Err(_) => return Err(bind_error),
    };
    if !metadata.file_type().is_socket() {
        return Err(bind_error);
    }

    match UnixStream::connect(path) {
        Ok(_) => Err(bind_error),
        Err(error) if error.kind() == ErrorKind::ConnectionRefused => {
            remove_unchanged_stale_socket(path, SocketIdentity::from_metadata(&metadata))?;
            UnixListener::bind(path)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => UnixListener::bind(path),
        Err(_) => Err(bind_error),
    }
}

fn remove_unchanged_stale_socket(path: &Path, stale_identity: SocketIdentity) -> io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_socket()
        || SocketIdentity::from_metadata(&metadata) != stale_identity
    {
        return Err(io::Error::new(
            ErrorKind::AddrInUse,
            "Unix socket endpoint changed during stale recovery",
        ));
    }
    std::fs::remove_file(path)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;

    use super::*;

    #[test]
    fn stale_socket_is_reclaimed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("service.sock");
        drop(UnixListener::bind(&path).unwrap());

        let listener = OwnedUnixListener::bind(&path).unwrap();

        assert!(UnixStream::connect(&path).is_ok());
        drop(listener);
        assert!(!path.exists());
    }

    #[test]
    fn live_socket_is_not_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("service.sock");
        let live = UnixListener::bind(&path).unwrap();

        let error = OwnedUnixListener::bind(&path).err().unwrap();

        assert_eq!(error.kind(), ErrorKind::AddrInUse);
        assert!(UnixStream::connect(&path).is_ok());
        drop(live);
    }

    #[test]
    fn non_socket_endpoint_is_not_reclaimed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("service.sock");
        std::fs::write(&path, b"owned by another endpoint kind").unwrap();

        let error = OwnedUnixListener::bind(&path).err().unwrap();

        assert_eq!(error.kind(), ErrorKind::AddrInUse);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"owned by another endpoint kind"
        );
    }

    #[test]
    fn cleanup_does_not_remove_replacement_socket() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("service.sock");
        let owned = OwnedUnixListener::bind(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let replacement = UnixListener::bind(&path).unwrap();

        drop(owned);

        assert!(UnixStream::connect(&path).is_ok());
        drop(replacement);
    }

    #[test]
    fn concurrent_binders_cannot_share_endpoint_ownership() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("service.sock");
        let barrier = Arc::new(Barrier::new(2));
        let first_path = path.clone();
        let first_barrier = Arc::clone(&barrier);
        let first = thread::spawn(move || {
            let listener = OwnedUnixListener::bind(first_path).unwrap();
            first_barrier.wait();
            first_barrier.wait();
            listener
        });
        barrier.wait();

        let error = OwnedUnixListener::bind(&path).err().unwrap();

        assert_eq!(error.kind(), ErrorKind::AddrInUse);
        assert!(UnixStream::connect(&path).is_ok());
        barrier.wait();
        drop(first.join().unwrap());
    }
}
