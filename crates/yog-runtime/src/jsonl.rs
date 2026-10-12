use std::{
    fs::File,
    io::{self, BufWriter, Write},
};

use serde::Serialize;

pub trait WriteLine {
    fn write_line(&mut self, value: &impl Serialize) -> io::Result<()>;
}

impl WriteLine for BufWriter<File> {
    fn write_line(&mut self, value: &impl Serialize) -> io::Result<()> {
        serde_json::to_writer(&mut *self, value)?;
        self.write_all(b"\n")?;
        self.flush()
    }
}
