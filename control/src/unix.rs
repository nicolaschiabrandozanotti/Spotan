//! Unix IPC adapter. Socket location is supplied by the caller.
use crate::{Result, Snapshot};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};
pub fn default_socket() -> PathBuf {
    if let Some(path) = std::env::var_os("SPOTI_SOCKET") {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(path).join("ncspot/ncspot.sock");
    }
    // Match ncspot's Unix runtime-directory fallback instead of guessing UID 1000.
    let uid = unsafe { libc::getuid() };
    let linux = PathBuf::from(format!("/run/user/{uid}"));
    if cfg!(target_os = "linux") && linux.is_dir() {
        linux.join("ncspot/ncspot.sock")
    } else {
        PathBuf::from(format!("/tmp/ncspot-{uid}/ncspot.sock"))
    }
}

fn connect(path: &Path) -> Result<BufReader<UnixStream>> {
    let socket = UnixStream::connect(path)?;
    socket.set_read_timeout(Some(Duration::from_millis(500)))?;
    socket.set_write_timeout(Some(Duration::from_millis(500)))?;
    Ok(BufReader::new(socket))
}
pub fn read(reader: &mut impl BufRead) -> Result<Snapshot> {
    let mut bytes = Vec::new();
    reader.take(262_144).read_until(b'\n', &mut bytes)?;
    if !bytes.ends_with(b"\n") {
        return Err("Incomplete Spoti snapshot".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
pub fn snapshot(path: &Path) -> Result<Snapshot> {
    read(&mut connect(path)?)
}
pub fn command(path: &Path, command: &str) -> Result<()> {
    if command.contains(['\n', '\r']) {
        return Err("Only one command is allowed".into());
    }
    let mut reader = connect(path)?;
    read(&mut reader)?; // Drain the greeting before closing, avoiding unread-data resets.
    writeln!(reader.get_mut(), "{command}")?;
    reader.get_ref().shutdown(Shutdown::Write)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Cursor, os::unix::net::UnixListener, thread};
    #[test]
    fn incomplete_and_oversized_greetings_are_rejected() {
        assert!(read(&mut Cursor::new(b"{\"mode\":\"Stopped\"}")).is_err());
        assert!(read(&mut Cursor::new(vec![b'x'; 300_000])).is_err());
    }
    #[test]
    fn command_drains_the_greeting_and_arrives_intact() {
        let path =
            std::env::temp_dir().join(format!("spoti-control-test-{}.sock", std::process::id()));
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .write_all(b"{\"mode\":\"Stopped\",\"playable\":null}\n")
                .unwrap();
            let mut line = String::new();
            BufReader::new(socket).read_line(&mut line).unwrap();
            line
        });
        command(&path, "volume 73.0000").unwrap();
        assert_eq!(server.join().unwrap(), "volume 73.0000\n");
        std::fs::remove_file(path).unwrap();
    }
}
