use crate::prelude::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct Client {
    client: crates_io_api::SyncClient,
    cache_dir: PathBuf,
}

fn is_fresh(path: &Path) -> Result<bool> {
    let metadata = fs::metadata(path)?;
    let created = metadata.created().or_else(|_e| metadata.modified())?;
    let now = std::time::SystemTime::now();
    Ok(((now - Duration::from_secs(60 * 60 * 24)) < created) && (created < now))
}

trait Cacheable: Sized {
    fn get_cache_path(base: &Path, name: &str, version: &str) -> PathBuf;
    fn fetch(client: &crates_io_api::SyncClient, crate_: &str, _version: &str) -> Result<Self>;
}

impl Cacheable for crates_io_api::CrateResponse {
    fn get_cache_path(base: &Path, name: &str, _version: &str) -> PathBuf {
        base.join("crate").join(format!("{}.json", name))
    }
    fn fetch(client: &crates_io_api::SyncClient, crate_: &str, _version: &str) -> Result<Self> {
        Ok(client.get_crate(crate_)?)
    }
}

impl Cacheable for crates_io_api::Owners {
    fn get_cache_path(base: &Path, name: &str, _version: &str) -> PathBuf {
        base.join("owners").join(format!("{}.json", name))
    }
    fn fetch(client: &crates_io_api::SyncClient, crate_: &str, _version: &str) -> Result<Self> {
        Ok(crates_io_api::Owners {
            users: client.crate_owners(crate_)?,
        })
    }
}

fn get_downloads_stats(resp: &crates_io_api::CrateResponse, version: &str) -> (u64, u64) {
    (
        resp.versions
            .iter()
            .find(|v| v.num == version)
            .map(|v| v.downloads)
            .unwrap_or(0),
        resp.crate_data.downloads,
    )
}

impl Client {
    pub fn new(local: &crev_lib::Local) -> Result<Self> {
        let cache_dir = local.get_root_cache_dir().join("crates_io");
        fs::create_dir_all(&cache_dir)?;
        Ok(Self {
            client: crates_io_api::SyncClient::new(),
            cache_dir: cache_dir,
        })
    }

    fn load_cache(&self, path: &Path) -> Result<String> {
        let mut file = std::fs::File::open(path)?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;

        Ok(content)
    }

    fn get_from_cache<T: Cacheable + DeserializeOwned>(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Option<(T, bool)>> {
        let path = T::get_cache_path(&self.cache_dir, name, version);
        if path.exists() {
            let content = self.load_cache(&path)?;
            let v = serde_json::from_str::<T>(&content)?;
            Ok(Some((v, is_fresh(&path)?)))
        } else {
            Ok(None)
        }
    }

    fn store_in_cache<T: Cacheable + Serialize>(&self, path: &Path, resp: &T) -> Result<()> {
        crev_common::store_to_file_with(&path, |file| serde_json::to_writer(file, &resp))??;
        Ok(())
    }

    fn fetch<T: Cacheable + Serialize>(&self, crate_: &str, version: &str) -> Result<T> {
        let resp = T::fetch(&self.client, crate_, version)?;
        self.store_in_cache(&T::get_cache_path(&self.cache_dir, crate_, version), &resp)?;
        Ok(resp)
    }

    fn get<T: Cacheable + DeserializeOwned + Serialize>(
        &self,
        crate_: &str,
        version: &str,
    ) -> Result<T> {
        let cached: Option<(T, bool)> = self.get_from_cache(crate_, version)?;

        match cached {
            Some((resp, true)) => Ok(resp),
            Some((resp, false)) => match self.fetch(crate_, version) {
                Ok(new_resp) => Ok(new_resp),
                Err(_e) => Ok(resp),
            },
            None => self.fetch(crate_, version),
        }
    }

    pub fn get_downloads_count(&self, crate_: &str, version: &str) -> Result<(u64, u64)> {
        Ok(get_downloads_stats(
            &self.get::<crates_io_api::CrateResponse>(crate_, version)?,
            version,
        ))
    }

    pub fn get_owners(&self, crate_: &str) -> Result<Vec<String>> {
        let owners = self.get::<crates_io_api::Owners>(crate_, "")?;
        Ok(owners.users.into_iter().map(|u| u.login).collect())
    }
}
