use std::{
    io,
    path::{Path, PathBuf},
};

use path_absolutize::Absolutize;

pub trait Absolute {
    fn absolutize(&self) -> String;

    fn try_absolutize(&self) -> io::Result<PathBuf>;
}

impl Absolute for Path {
    fn absolutize(&self) -> String {
        self.try_absolutize()
            .unwrap_or_else(|_| self.to_path_buf())
            .to_string_lossy()
            .into_owned()
    }

    fn try_absolutize(&self) -> io::Result<PathBuf> {
        self.canonicalize()
            .or_else(|_| <Self as Absolutize>::absolutize(self).map(|path| path.into_owned()))
    }
}
