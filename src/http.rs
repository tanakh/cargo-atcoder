use anyhow::{anyhow, Result};
use itertools::Itertools;
use reqwest::{
    cookie::{CookieStore, Jar},
    header::HeaderValue,
    Client as ReqwestClient, Url,
};
use std::{
    fs::File,
    io::{BufRead, BufReader, Write as _},
    path::{Path, PathBuf},
    sync::Arc,
};

pub struct Client {
    client: ReqwestClient,
    cookie_store: Arc<Jar>,
    session_file: PathBuf,
    endpoint: String,
}

impl Drop for Client {
    fn drop(&mut self) {
        let result = (|| -> anyhow::Result<()> {
            let mut file = File::create(&self.session_file)
                .map_err(|e| anyhow!("failed to open `{}`: {}", self.session_file.display(), e))?;

            for cookie in self
                .cookie_store
                .cookies(&self.endpoint.parse::<Url>().unwrap())
            {
                writeln!(&mut file, "{}", cookie.to_str()?)?;
            }

            Ok(())
        })();

        if let Err(err) = result {
            let _ = eprintln!("An error occurred while saving the session: {}", err);
        }
    }
}

fn load_cookie_store(session_file: &Path, endpoint: &str) -> Result<Jar> {
    let url = endpoint.parse().unwrap();
    let jar = reqwest::cookie::Jar::default();
    let f = File::open(session_file);

    if f.is_err() {
        return Ok(jar);
    }

    for line in BufReader::new(f?).lines() {
        let v = line?
            .split("; ")
            .map(|s| HeaderValue::from_str(s).unwrap())
            .collect_vec();
        jar.set_cookies(&mut v.iter(), &url)
    }
    Ok(jar)
}

impl Client {
    pub fn new(session_file: &Path, endpoint: &str) -> Result<Self> {
        static USER_AGENT: &str = "cargo-atcoder";

        let cookie_store = Arc::new(load_cookie_store(session_file, endpoint)?);

        let client = reqwest::ClientBuilder::new()
            .cookie_provider(cookie_store.clone())
            .user_agent(USER_AGENT)
            .build()?;

        Ok(Self {
            client,
            cookie_store,
            session_file: session_file.to_owned(),
            endpoint: endpoint.to_owned(),
        })
    }

    pub async fn get(&self, url: &Url) -> Result<String> {
        let resp = self.client.get(url.clone()).send();
        Ok(resp.await?.error_for_status()?.text().await?)
    }

    pub async fn post_form(&self, url: &Url, form: &[(&str, &str)]) -> Result<String> {
        let resp = self.client.post(url.clone()).form(form).send();
        Ok(resp.await?.error_for_status()?.text().await?)
    }
}

pub fn is_http_error(err: &anyhow::Error, status_code: reqwest::StatusCode) -> bool {
    matches!(
        err.downcast_ref::<reqwest::Error>(),
        Some(err) if err.status() == Some(status_code),
    )
}
