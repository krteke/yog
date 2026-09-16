use rustix::fs::{CWD, RenameFlags, renameat_with};
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

pub struct Output {
    part: PathBuf,
    target: PathBuf,
    overwrite: bool,
    published: bool,
}

impl Output {
    pub fn prepare(target: &Path, overwrite: bool) -> io::Result<Self> {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        if target.exists() && !overwrite {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "output file already exists",
            ));
        }

        let target = target.to_path_buf();
        let part = target.with_added_extension("part");

        File::create_new(&part).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("cannot create {}: {error}", part.display()),
            )
        })?;

        Ok(Self {
            part,
            target,
            overwrite,
            published: false,
        })
    }

    pub fn part(&self) -> &Path {
        &self.part
    }

    pub fn publish(mut self) -> io::Result<()> {
        if self.part.metadata()?.len() == 0 {
            return Err(io::Error::other("generated output file is empty"));
        }
        if self.overwrite {
            fs::rename(&self.part, &self.target)?;
        } else {
            renameat_with(CWD, &self.part, CWD, &self.target, RenameFlags::NOREPLACE)?;
        }
        self.published = true;

        Ok(())
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(&self.part);
        }
    }
}
