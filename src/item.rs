use std::ffi::OsStr;
use std::fs::{self, FileType, Metadata};
use std::io;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Form {
    File,
    Dir,
    Link,
    Other,
}

impl Form {
    pub fn of(kind: FileType) -> Form {
        if kind.is_dir() {
            Form::Dir
        } else if kind.is_file() {
            Form::File
        } else if kind.is_symlink() {
            Form::Link
        } else {
            Form::Other
        }
    }

    pub fn is_dir(self) -> bool {
        self == Form::Dir
    }

    pub fn is_file(self) -> bool {
        self == Form::File
    }

    pub fn is_symlink(self) -> bool {
        self == Form::Link
    }
}

#[derive(Clone, Copy)]
pub struct Item<'a> {
    pub path: &'a Path,
    pub depth: usize,
    pub form: Form,
    pub follow: bool,
}

impl Item<'_> {
    pub fn path(&self) -> &Path {
        self.path
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    pub fn file_name(&self) -> &OsStr {
        self.path.file_name().unwrap_or(self.path.as_os_str())
    }

    pub fn metadata(&self) -> io::Result<Metadata> {
        if self.follow
            && let Ok(metadata) = fs::metadata(self.path)
        {
            return Ok(metadata);
        }
        fs::symlink_metadata(self.path)
    }
}
