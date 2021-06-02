use anyhow::{anyhow, Result};
use reqwest::{
    blocking::Client as ReqwestClient,
    cookie::{CookieStore, Jar},
    Url,
};
use std::{
    fs::File,
    io::{self, BufRead, BufReader, Write as _},
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
                eprintln!("cookie: {:?}", cookie);

                writeln!(&mut file, "{}", cookie.to_str()?)?;
            }

            Ok(())
        })();

        if let Err(err) = result {
            let _ = writeln!(
                io::stderr(),
                "An error occurred while saving the session: {}",
                err,
            );
        }
    }
}

fn load_cookie_store(session_file: &Path, endpoint: &str) -> Result<Jar> {
    let url = endpoint.parse().unwrap();
    let jar = reqwest::cookie::Jar::default();
    let f = File::open(session_file);

    if f.is_err() {
        eprintln!("Session file not found. start new session.2");
        return Ok(jar);
    }

    for line in BufReader::new(f.unwrap()).lines() {
        jar.add_cookie_str(&line?, &url);
    }
    Ok(jar)
}

impl Client {
    pub fn new(session_file: &Path, endpoint: &str) -> Result<Self> {
        static USER_AGENT: &str = "cargo-atcoder";

        let cookie_store = Arc::new(load_cookie_store(session_file, endpoint)?);

        let client = reqwest::blocking::ClientBuilder::new()
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

    pub fn get(&self, url: &Url) -> anyhow::Result<String> {
        Ok(self.client.get(url.clone()).send()?.text()?)
    }

    pub fn post_form(&self, url: &Url, form: &[(&str, &str)]) -> anyhow::Result<String> {
        Ok(self.client.post(url.clone()).form(form).send()?.text()?)
    }
}
