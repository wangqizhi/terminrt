pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

#[cfg(windows)]
mod platform {
    use std::io::{self, Read, Write};

    /// Readable end of the PTY — goes to the background reader thread.
    pub struct PtyReader {
        reader: conpty::io::PipeReader,
    }

    unsafe impl Send for PtyReader {}

    impl PtyReader {
        pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.reader.read(buf)
        }
    }

    /// Writable end + process handle — stays on the main thread.
    pub struct PtyWriter {
        #[allow(dead_code)]
        process: conpty::Process,
        writer: conpty::io::PipeWriter,
    }

    impl PtyWriter {
        pub fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
            self.writer.write_all(data)
        }

        pub fn resize(&mut self, size: super::PtySize) -> io::Result<()> {
            self.process
                .resize(size.cols as i16, size.rows as i16)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))
        }
    }

    pub fn spawn(size: super::PtySize) -> io::Result<(PtyReader, PtyWriter)> {
        let mut process = conpty::ProcessOptions::default()
            .set_console_size(Some((size.cols as i16, size.rows as i16)))
            .spawn(std::process::Command::new("powershell.exe"))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

        let reader = process
            .output()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        let writer = process
            .input()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

        Ok((PtyReader { reader }, PtyWriter { process, writer }))
    }
}

#[cfg(not(windows))]
mod platform {
    use std::io;

    pub struct PtyReader;

    impl PtyReader {
        pub fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            unimplemented!("PTY not yet implemented for this platform")
        }
    }

    pub struct PtyWriter;

    impl PtyWriter {
        pub fn write_all(&mut self, _data: &[u8]) -> io::Result<()> {
            unimplemented!("PTY not yet implemented for this platform")
        }

        pub fn resize(&mut self, _size: super::PtySize) -> io::Result<()> {
            unimplemented!("PTY not yet implemented for this platform")
        }
    }

    pub fn spawn(_size: super::PtySize) -> io::Result<(PtyReader, PtyWriter)> {
        // TODO: implement Unix PTY (e.g. using nix or rustix)
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "PTY not yet implemented for this platform",
        ))
    }
}

pub use platform::{PtyReader, PtyWriter};
pub use platform::spawn as spawn_pty;
