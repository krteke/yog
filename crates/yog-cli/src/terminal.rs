use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use std::io;

pub struct NonblockingStderr(OFlags);

impl NonblockingStderr {
    pub fn new() -> io::Result<Self> {
        let stderr = io::stderr();
        let flags = fcntl_getfl(&stderr)?;
        fcntl_setfl(&stderr, flags | OFlags::NONBLOCK)?;
        Ok(Self(flags))
    }
}

impl Drop for NonblockingStderr {
    fn drop(&mut self) {
        let _ = fcntl_setfl(io::stderr(), self.0);
    }
}
