use anyhow::{anyhow, bail, ensure, Context as _, Result};
use cookie_store::CookieStore;
use itertools::Itertools as _;
use std::fs::{self, File};
use std::io::{self, Cursor, Write as _};
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use thiserror::Error;
use ureq_1_0_0::{Agent, Request, Response};
use url::Url;

const REDIRECTS: u32 = 5;

const _ASSERT_THE_UREQ_COOKIE_FEATURE_IS_DISABLED: () = {
    enum AssertTheAssocFnAgentCookieIsNotImplemented {}

    trait AgentExt {
        fn cookie(&self) -> AssertTheAssocFnAgentCookieIsNotImplemented {
            unreachable!();
        }
    }

    impl AgentExt for Agent {}

    let _: fn(&Agent) -> AssertTheAssocFnAgentCookieIsNotImplemented = Agent::cookie;
};

#[derive(Error, Debug)]
#[error("{url}: {status} {status_text}")]
pub(crate) struct StatusError {
    url: String,
    status: u16,
    status_text: String,
}

impl StatusError {
    fn new(res: &Response) -> Self {
        Self {
            url: res.get_url().to_owned(),
            status: res.status(),
            status_text: res.status_text().to_owned(),
        }
    }

    pub(crate) fn status(&self) -> u16 {
        self.status
    }
}

pub struct Client {
    agent: Agent,
    cookie_store: Arc<Mutex<CookieStore>>,
    session_file: PathBuf,
}

impl Drop for Client {
    fn drop(&mut self) {
        let result = (|| -> _ {
            let mut file = File::create(&self.session_file)
                .map_err(|e| format!("failed to open `{}`: {}", self.session_file.display(), e))?;

            self.cookie_store
                .lock()
                .map_err(|e| e.to_string())?
                .save_json(&mut file)
                .map_err(|e| e.to_string())
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

impl Client {
    pub fn new(session_file: &Path) -> Result<Self> {
        static USER_AGENT: &str = "cargo-atcoder";

        let agent = ureq_1_0_0::agent().set("User-Agent", USER_AGENT).build();

        let cookie_store = Arc::new(Mutex::new(if session_file.exists() {
            let jsonl = fs::read_to_string(&session_file)
                .with_context(|| format!("Failed to read `{}`", session_file.display()))?;
            CookieStore::load_json(Cursor::new(jsonl.into_bytes())).map_err(|e| anyhow!("{}", e))?
        } else {
            CookieStore::default()
        }));

        Ok(Self {
            agent,
            cookie_store,
            session_file: session_file.to_owned(),
        })
    }

    pub fn get(&self, url: &Url) -> anyhow::Result<String> {
        self.req("GET", url, Request::call)
    }

    pub fn post_form(&self, url: &Url, form: &[(&str, &str)]) -> anyhow::Result<String> {
        self.req("POST", url, |r| r.send_form(form))
    }

    fn req(
        &self,
        mut method: &str,
        url: &Url,
        call: impl FnOnce(&mut Request) -> Response,
    ) -> anyhow::Result<String> {
        let mut call = Box::new(call) as Box<dyn FnOnce(&mut _) -> _>;

        // Redirects manually.
        let mut rest_redirects = REDIRECTS;
        loop {
            let mut req = self.agent.request(method, url.as_ref());
            req.redirects(0);

            // https://github.com/seanmonstar/reqwest/pull/522
            // https://github.com/seanmonstar/reqwest/pull/539
            let cookie_header = self
                .cookie_store
                .lock()
                .unwrap()
                .get_request_cookies(&url)
                .map(|c| format!("{}={}", c.name(), c.value()))
                .join("; ");

            if !cookie_header.is_empty() {
                req.set("Cookie", &cookie_header);
            }

            let res = mem::replace(&mut call, Box::new(Request::call))(&mut req);

            if let Some(err) = res.synthetic_error() {
                let mut err = err as &dyn std::error::Error;
                let mut displays = vec![err.to_string()];
                while let Some(source) = err.source() {
                    displays.push(source.to_string());
                    err = source;
                }
                let mut displays = displays.into_iter().rev();
                let cause = anyhow!("{}", displays.next().unwrap());
                return Err(displays.fold(cause, |err, display| err.context(display)));
            }

            if res.error() {
                return Err(StatusError::new(&res).into());
            }

            let cookies = res
                .all("Set-Cookie")
                .into_iter()
                .map(str::parse)
                .collect::<std::result::Result<Vec<cookie_0_12_0::Cookie<'_>>, _>>()?
                .into_iter();
            self.cookie_store
                .lock()
                .unwrap()
                .store_response_cookies(cookies, &url);

            if !(300..=399).contains(&res.status()) {
                break res.into_string().map_err(Into::into);
            }

            if rest_redirects == 0 {
                bail!("{}: too many redirects", res.get_url());
            }

            let location = res.header("Location").with_context(|| {
                format!(
                    "{}: `{} {}` without `Location`",
                    res.get_url(),
                    res.status(),
                    res.status_text(),
                )
            })?;
            let url = url.join(location)?;
            method = "GET";

            ensure!(url.host_str() == Some("atcoder.jp"), "cross host");

            rest_redirects -= 1;
        }
    }
}
