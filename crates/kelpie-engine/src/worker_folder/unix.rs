//! The Unix implementation: `openat` with `O_NOFOLLOW`, as the module above describes

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::OwnedFd;
use std::path::{Component, Path};

use rustix::fs::{AtFlags, FileType, Mode, OFlags, fstat, mkdirat, openat, renameat, unlinkat};
use rustix::io::Errno;

use super::{Error, MAX};

impl From<Errno> for Error {
    fn from(e: Errno) -> Self {
        Self::Io(io::Error::from(e).kind())
    }
}

pub(super) fn read(root: &Path, rel: &Path) -> Result<Option<String>, Error> {
    let Some((dir, name)) = walk(root, rel, false)? else {
        return Ok(None);
    };
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY;
    let fd = match openat(&dir, &name, flags | OFlags::CLOEXEC, Mode::empty()) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(None),
        Err(Errno::LOOP | Errno::NOTDIR) => return Err(Error::NotRegular),
        Err(e) => return Err(e.into()),
    };
    let stat = fstat(&fd)?;
    let regular = FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile;
    // A file with another name too is a hard link, which O_NOFOLLOW cannot see: the
    // worker could link a file of the maintainer's into its folder.
    let ours = stat.st_uid == rustix::process::geteuid().as_raw() && stat.st_nlink == 1;
    if !regular || !ours {
        return Err(Error::NotRegular);
    }
    if u64::try_from(stat.st_size).map_or(true, |size| size > MAX) {
        return Err(Error::TooBig);
    }
    let mut text = String::new();
    // One byte past the cap, so a file that grew since `fstat` is caught.
    File::from(fd)
        .take(MAX + 1)
        .read_to_string(&mut text)
        .map_err(|e| Error::Io(e.kind()))?;
    if text.len() as u64 > MAX {
        return Err(Error::TooBig);
    }
    Ok(Some(text))
}

pub(super) fn write(root: &Path, rel: &Path, text: &str, private: bool) -> Result<(), Error> {
    let mode = if private {
        Mode::RUSR | Mode::WUSR
    } else {
        Mode::from_bits_truncate(0o666)
    };
    put(root, rel, text, mode)
}

fn put(root: &Path, rel: &Path, text: &str, mode: Mode) -> Result<(), Error> {
    std::fs::create_dir_all(root).map_err(|e| Error::Io(e.kind()))?;
    let (dir, name) = walk(root, rel, true)?.ok_or(Error::Io(io::ErrorKind::NotFound))?;
    let mut beside = name.clone();
    beside.push(".tmp");
    let flags = OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let fd = match openat(&dir, &beside, flags, mode) {
        Ok(fd) => fd,
        // A file or link left there, by a crash or the worker. Removing it
        // removes a link itself, never what it names.
        Err(Errno::EXIST) => {
            unlinkat(&dir, &beside, AtFlags::empty())?;
            openat(&dir, &beside, flags, mode)?
        }
        Err(e) => return Err(e.into()),
    };
    let mut file = File::from(fd);
    let written = file.write_all(text.as_bytes());
    drop(file);
    written
        .map_err(|e| Error::Io(e.kind()))
        .and_then(|()| Ok(renameat(&dir, &beside, &dir, &name)?))
        .inspect_err(|_| {
            let _ = unlinkat(&dir, &beside, AtFlags::empty());
        })
}

// The folder holding `rel` below `root`, opened by descriptor, and the file's
// name in it. None when a folder on the way is not there and `make` is false.
fn walk(root: &Path, rel: &Path, make: bool) -> Result<Option<(OwnedFd, OsString)>, Error> {
    let mut names = Vec::new();
    for component in rel.components() {
        match component {
            Component::Normal(name) => names.push(name),
            _ => return Err(Error::Io(io::ErrorKind::InvalidInput)),
        }
    }
    let Some((file, folders)) = names.split_last() else {
        return Err(Error::Io(io::ErrorKind::InvalidInput));
    };
    let mut dir = match rustix::fs::open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(dir) => dir,
        Err(Errno::NOENT) if !make => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    for folder in folders {
        dir = match enter(&dir, folder, make)? {
            Some(next) => next,
            None => return Ok(None),
        };
    }
    Ok(Some((dir, (*file).to_owned())))
}

// `name` in `dir`, as a folder, made first when `make`; a link there is refused
fn enter(dir: &OwnedFd, name: &OsStr, make: bool) -> Result<Option<OwnedFd>, Error> {
    if make {
        match mkdirat(dir, name, Mode::from_bits_truncate(0o777)) {
            Ok(()) | Err(Errno::EXIST) => {}
            Err(e) => return Err(e.into()),
        }
    }
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    match openat(dir, name, flags, Mode::empty()) {
        Ok(next) => Ok(Some(next)),
        Err(Errno::NOENT) => Ok(None),
        Err(Errno::LOOP | Errno::NOTDIR) => Err(Error::NotFolder),
        Err(e) => Err(e.into()),
    }
}
