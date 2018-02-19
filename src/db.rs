use std::fs;
use std::path::{Path, PathBuf};

use failure::Error;
use tinycdb::{Cdb, CdbCreator};

pub fn open(path: &Path) -> Result<Option<Box<Cdb>>, Error> {
    if path.exists() {
        Cdb::open(&path)
            .map(Some)
            .map_err(|e| format_err!("DB couldn't be opened: {:?}", e))
    } else {
        Ok(None)
    }
}

pub fn data(guild: &str) -> PathBuf {
    PathBuf::from(format!("./data/{}", guild))
}
pub fn ensure_dir(dir: &Path) -> Result<(), Error> {
    if !dir.exists() {
        fs::create_dir_all(dir).map_err(|e| format_err!("Can't create directory: {}", e))
    } else {
        Ok(())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Guild {
    Colors,
    Users,
}
impl Guild {
    pub fn name(self) -> &'static str {
        match self {
            Guild::Colors => "colors",
            Guild::Users => "users",
        }
    }
    pub fn path(self, guild: &Path) -> PathBuf {
        match self {
            Guild::Colors => guild.join("colors.cdb"),
            Guild::Users => guild.join("users.cdb"),
        }
    }
    pub fn tmp_path(self, guild: &Path) -> PathBuf {
        match self {
            Guild::Colors => guild.join("colors.cdb.tmp"),
            Guild::Users => guild.join("users.cdb.tmp"),
        }
    }
    pub fn open(self, guild: &Path) -> Result<Option<Box<Cdb>>, Error> {
        open(&self.path(guild))
    }
    pub fn rm_tmp(self, guild: &Path) -> Result<(), Error> {
        let tmp_path = self.tmp_path(guild);
        if tmp_path.exists() {
            fs::remove_file(&tmp_path)
                .map_err(|e| format_err!("Couldn't remove old tmp {} DB: {}", self.name(), e))
        } else {
            Ok(())
        }
    }
    pub fn set<C, F, T>(self, guild: &Path, creator: C, f: F) -> Result<T, Error>
    where
        C: FnMut(&mut CdbCreator),
        F: FnOnce(Box<Cdb>) -> T,
    {
        let tmp_path = self.tmp_path(guild);
        let out = Cdb::new(&tmp_path, creator)
            .map(f)
            .map_err(|e| format_err!("Error creating {} DB: {:?}", self.name(), e))?;
        fs::rename(&tmp_path, &self.path(guild))
            .map_err(|e| format_err!("Couldn't replace old {} DB: {}", self.name(), e))?;
        Ok(out)
    }
}
