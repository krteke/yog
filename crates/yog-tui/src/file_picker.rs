use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

use crossterm::event::{KeyCode, KeyEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Directory,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathSelection {
    path: PathBuf,
    kind: EntryKind,
}

impl PathSelection {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn recursive(&self) -> bool {
        self.kind == EntryKind::Directory
    }

    pub fn display(&self) -> String {
        let mut value = self.path.display().to_string();
        if self.recursive() && !value.ends_with(std::path::MAIN_SEPARATOR) {
            value.push(std::path::MAIN_SEPARATOR);
        }
        value
    }
}

#[derive(Debug)]
pub struct Entry {
    path: PathBuf,
    name: OsString,
    kind: EntryKind,
}

impl Entry {
    pub fn name(&self) -> &OsString {
        &self.name
    }

    pub fn is_directory(&self) -> bool {
        self.kind == EntryKind::Directory
    }
}

pub enum PickerAction {
    None,
    Cancel,
    Confirm(PathSelection),
}

pub struct FilePicker {
    directory: PathBuf,
    entries: Vec<Entry>,
    cursor: usize,
    selected: Option<usize>,
    error: Option<String>,
}

impl FilePicker {
    pub fn open(current: Option<&PathSelection>) -> io::Result<Self> {
        let directory = match current {
            Some(selection) => selection
                .path()
                .parent()
                .unwrap_or_else(|| selection.path())
                .to_path_buf(),
            None => std::env::current_dir()?,
        };
        let entries = read_entries(&directory)?;
        let selected = current.and_then(|selection| {
            entries
                .iter()
                .position(|entry| entry.path == selection.path)
        });

        Ok(Self {
            directory,
            entries,
            cursor: selected.unwrap_or(0),
            selected,
            error: None,
        })
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> PickerAction {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => PickerAction::Cancel,
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_cursor(1);
                PickerAction::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_cursor(-1);
                PickerAction::None
            }
            KeyCode::Char('g') | KeyCode::Home => {
                self.cursor = 0;
                PickerAction::None
            }
            KeyCode::Char('G') | KeyCode::End => {
                self.cursor = self.entries.len().saturating_sub(1);
                PickerAction::None
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.open_parent();
                PickerAction::None
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.open_directory();
                PickerAction::None
            }
            KeyCode::Char(' ') => {
                self.toggle_selection();
                PickerAction::None
            }
            KeyCode::Enter => self.confirm(),
            _ => PickerAction::None,
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn move_cursor(&mut self, direction: isize) {
        if self.entries.is_empty() {
            return;
        }
        self.cursor = step(self.cursor, self.entries.len(), direction);
        self.error = None;
    }

    fn toggle_selection(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected = (self.selected != Some(self.cursor)).then_some(self.cursor);
        self.error = None;
    }

    fn confirm(&self) -> PickerAction {
        let Some(entry) = self.selected.and_then(|index| self.entries.get(index)) else {
            return PickerAction::None;
        };
        PickerAction::Confirm(PathSelection {
            path: entry.path.clone(),
            kind: entry.kind,
        })
    }

    fn open_parent(&mut self) {
        let Some(parent) = self.directory.parent().map(Path::to_path_buf) else {
            return;
        };
        self.change_directory(parent);
    }

    fn open_directory(&mut self) {
        let Some(entry) = self.entries.get(self.cursor) else {
            return;
        };
        if entry.is_directory() {
            self.change_directory(entry.path.clone());
        }
    }

    fn change_directory(&mut self, directory: PathBuf) {
        match read_entries(&directory) {
            Ok(entries) => {
                self.directory = directory;
                self.entries = entries;
                self.cursor = 0;
                self.selected = None;
                self.error = None;
            }
            Err(error) => {
                self.error = Some(format!("Cannot open {}: {error}", directory.display()));
            }
        }
    }
}

fn read_entries(directory: &Path) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = match fs::metadata(&path) {
            Ok(metadata) => metadata.file_type(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let kind = if file_type.is_dir() {
            EntryKind::Directory
        } else if file_type.is_file() {
            EntryKind::File
        } else {
            continue;
        };
        entries.push(Entry {
            path,
            name: entry.file_name(),
            kind,
        });
    }
    entries.sort_by(|left, right| {
        right
            .is_directory()
            .cmp(&left.is_directory())
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(entries)
}

fn step(current: usize, len: usize, direction: isize) -> usize {
    debug_assert!(len > 0);
    if direction < 0 {
        current.checked_sub(1).unwrap_or(len - 1)
    } else {
        (current + 1) % len
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(value: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(value), KeyModifiers::NONE)
    }

    #[test]
    fn selection_type_determines_recursive_mode() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("folder");
        let file = root.path().join("file.mkv");
        fs::create_dir(&folder).unwrap();
        fs::write(&file, []).unwrap();
        let mut picker = FilePicker {
            directory: root.path().to_path_buf(),
            entries: read_entries(root.path()).unwrap(),
            cursor: 0,
            selected: None,
            error: None,
        };

        assert!(picker.entries[0].is_directory());
        assert!(matches!(picker.handle_key(key(' ')), PickerAction::None));
        let PickerAction::Confirm(selection) =
            picker.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        else {
            panic!("selected directory was not confirmed");
        };

        assert_eq!(selection.path(), folder);
        assert!(selection.recursive());

        picker.handle_key(key('j'));
        picker.handle_key(key(' '));
        let PickerAction::Confirm(selection) =
            picker.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        else {
            panic!("selected file was not confirmed");
        };

        assert_eq!(selection.path(), file);
        assert!(!selection.recursive());
    }

    #[test]
    fn entering_a_directory_resets_the_selection() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("folder");
        fs::create_dir(&folder).unwrap();
        fs::create_dir(folder.join("nested")).unwrap();
        let mut picker = FilePicker {
            directory: root.path().to_path_buf(),
            entries: read_entries(root.path()).unwrap(),
            cursor: 0,
            selected: None,
            error: None,
        };
        picker.handle_key(key(' '));

        picker.handle_key(key('l'));

        assert_eq!(picker.directory(), folder);
        assert_eq!(picker.entries().len(), 1);
        assert!(picker.entries()[0].is_directory());
        assert_eq!(picker.selected(), None);
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_does_not_make_its_directory_unreadable() {
        use std::{ffi::OsStr, os::unix::fs::symlink};

        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("video.mkv"), []).unwrap();
        symlink(root.path().join("missing"), root.path().join("dangling")).unwrap();

        let entries = read_entries(root.path()).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name(), OsStr::new("video.mkv"));
    }
}
