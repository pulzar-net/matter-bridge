//! Small file helpers: atomic writes, owner-only permissions.

use std::io::Write;
use std::path::Path;

use serde::de::DeserializeOwned;

use crate::Error;

/// Read a JSON file, or `None` if it does not exist.
pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, Error> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| Error::Store(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Io(e)),
    }
}

/// Replace `path` with `data` atomically: a reader sees the old file or the
/// new one, never half of either, and a power cut leaves one of the two.
pub(crate) fn write_atomic(path: &Path, data: &[u8], private: bool) -> Result<(), Error> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;

    let tmp = path.with_extension("tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = private;
        let mut file = opts.open(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    if let Ok(dir) = std::fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// [`write_atomic`] for a file holding keys.
pub(crate) fn write_private(path: &Path, data: &[u8]) -> Result<(), Error> {
    write_atomic(path, data, true)
}
