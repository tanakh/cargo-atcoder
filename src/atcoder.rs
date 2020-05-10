use anyhow::{anyhow, bail, ensure, Context as _, Result};
use chrono::{DateTime, Utc};
use cookie_store::CookieStore;
use itertools::Itertools as _;
use regex::Regex;
use scraper::{element_ref::ElementRef, Html, Selector};
use std::fs::{self, File};
use std::io::{self, Cursor, Write as _};
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use thiserror::Error;
use ureq_1_0_0::{Agent, Request, Response};
use url::Url;

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

pub struct AtCoder {
    agent: Agent,
    cookie_store: Arc<Mutex<CookieStore>>,
    session_file: PathBuf,
}

#[derive(Debug)]
pub struct ContestInfo {
    problems: Vec<Problem>,
}

#[derive(Debug)]
pub struct Problem {
    pub id: String,
    pub name: String,
    pub url: String,
    pub tle: String,
    pub mle: String,
}

#[derive(Debug, Clone)]
pub struct TestCase {
    pub input: String,
    pub output: String,
}

impl ContestInfo {
    pub fn problem(&self, id: &str) -> Option<&Problem> {
        self.problems
            .iter()
            .find(|p| p.id.to_lowercase() == id.to_lowercase())
    }

    pub fn problem_ids_lowercase(&self) -> Vec<String> {
        self.problems.iter().map(|p| p.id.to_lowercase()).collect()
    }
}

#[derive(Debug)]
pub struct SubmissionResult {
    pub id: usize,
    pub date: DateTime<Utc>,
    pub problem_name: String,
    pub user: String,
    pub language: String,
    pub score: i64,
    pub code_length: String,
    pub status: StatusCode,
    pub run_time: Option<String>,
    pub memory: Option<String>,
}

#[derive(Debug)]
pub struct FullSubmissionResult {
    pub result: SubmissionResult,
    pub cases: Vec<CaseResult>,
}

#[derive(Debug)]
pub struct CaseResult {
    pub name: String,
    pub result: StatusCode,
    pub run_time: Option<String>,
    pub memory: Option<String>,
}

#[derive(Debug)]
pub enum StatusCode {
    Waiting(WaitingCode),
    Progress(usize, usize, Option<ResultCode>),
    Done(ResultCode),
}

impl StatusCode {
    pub fn done(&self) -> bool {
        match self {
            StatusCode::Done(_) => true,
            _ => false,
        }
    }

    pub fn result_code(&self) -> Option<&ResultCode> {
        match self {
            StatusCode::Done(code) => Some(code),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum WaitingCode {
    WaitingForJudge,
    WaitingForRejudge,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResultCode {
    Accepted,
    WrongAnswer,
    TimeLimitExceeded,
    MemoryLimitExceeded,
    OutputLimitExceeded,
    RuntimeError,
    CompileError,
    InternalError,
    Unknown(String),
}

impl ResultCode {
    pub fn short_msg(&self) -> String {
        use ResultCode::*;
        match self {
            CompileError => "CE".to_string(),
            MemoryLimitExceeded => "MLE".to_string(),
            TimeLimitExceeded => "TLE".to_string(),
            RuntimeError => "RE".to_string(),
            OutputLimitExceeded => "OLE".to_string(),
            InternalError => "IE".to_string(),
            WrongAnswer => "WA".to_string(),
            Accepted => "AC".to_string(),
            Unknown(s) => format!("UNK({})", s),
        }
    }

    pub fn long_msg(&self) -> String {
        use ResultCode::*;
        match self {
            CompileError => "Compile Error".to_string(),
            MemoryLimitExceeded => "Memory Limit Exceeded".to_string(),
            TimeLimitExceeded => "Time Limit Exceeded".to_string(),
            RuntimeError => "Runtime Error".to_string(),
            OutputLimitExceeded => "Output Limit Exceeded".to_string(),
            InternalError => "Internal Error".to_string(),
            WrongAnswer => "Wrong Answer".to_string(),
            Accepted => "Accepted".to_string(),
            Unknown(code) => format!("Unknown ({})", code),
        }
    }

    pub fn accepted(&self) -> bool {
        use ResultCode::*;
        match self {
            Accepted => true,
            _ => false,
        }
    }
}

impl StatusCode {
    fn from_str(s: &str) -> Option<StatusCode> {
        use ResultCode::*;
        use StatusCode::*;
        use WaitingCode::*;

        match s {
            "WJ" => return Some(Waiting(WaitingForJudge)),
            "WR" => return Some(Waiting(WaitingForRejudge)),
            _ => (),
        }

        // In progress, result code is as below:
        // 6/9 TLE

        let re = Regex::new(r"^(\d+) */ *(\d+) *(.*)$").unwrap();

        if let Some(caps) = re.captures(s) {
            let cur = caps[1].parse().unwrap();
            let total = caps[2].parse().unwrap();

            let rest = caps[3].trim();
            if rest == "" {
                return Some(Progress(cur, total, None));
            }

            let code = Self::from_str(rest)?;
            if let Done(code) = code {
                return Some(Progress(cur, total, Some(code)));
            } else {
                panic!("Invalid result status code: `{}`", s);
            }
        }

        Some(Done(match s {
            "CE" => CompileError,
            "MLE" => MemoryLimitExceeded,
            "TLE" => TimeLimitExceeded,
            "RE" => RuntimeError,
            "OLE" => OutputLimitExceeded,
            "IE" => InternalError,
            "WA" => WrongAnswer,
            "AC" => Accepted,
            _ => Unknown(s.to_owned()),
        }))
    }
}

impl Drop for AtCoder {
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

impl AtCoder {
    pub fn new(session_file: &Path) -> Result<AtCoder> {
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

    fn check_login(&self) -> Result<()> {
        let _ = self
            .username()?
            .with_context(|| "You are not logged in. Please login first.")?;
        Ok(())
    }

    pub fn username(&self) -> Result<Option<String>> {
        let doc = self.http_get("/")?;
        let doc = Html::parse_document(&doc);

        let r = doc
            .select(&Selector::parse("li a[href^=\"/users/\"]").unwrap())
            .next();

        if r.is_none() {
            return Ok(None);
        }

        Ok(Some(
            r.unwrap().value().attr("href").unwrap()[7..].to_owned(),
        ))
    }

    pub fn login(&self, username: &str, password: &str) -> Result<()> {
        let document = self.http_get("/login")?;
        let document = Html::parse_document(&document);

        let csrf_token = document
            .select(&Selector::parse("input[name=\"csrf_token\"]").unwrap())
            .next()
            .with_context(|| "cannot find csrf_token")?;

        let csrf_token = csrf_token
            .value()
            .attr("value")
            .with_context(|| "cannot find csrf_token")?;

        let res = self.http_post_form(
            "/login",
            &[
                ("username", username),
                ("password", password),
                ("csrf_token", csrf_token),
            ],
        )?;

        let res = Html::parse_document(&res);

        // On failure:
        // <div class="alert alert-danger alert-dismissible col-sm-12 fade in" role="alert">
        //   ...
        //   {{error message}}
        // </div>
        if let Some(err) = res
            .select(&Selector::parse("div.alert-danger").unwrap())
            .next()
        {
            bail!(
                "Login failed: {}",
                err.last_child().unwrap().value().as_text().unwrap().trim()
            );
        }

        // On success:
        // <div class="alert alert-success alert-dismissible col-sm-12 fade in" role="alert" >
        //     ...
        //     ようこそ、tanakh さん。
        // </div>
        if res
            .select(&Selector::parse("div.alert-success").unwrap())
            .next()
            .is_some()
        {
            return Ok(());
        }

        Err(anyhow!("Login failed: Unknown error"))
    }

    pub fn problem_ids_from_score_table(&self, contest_id: &str) -> Result<Option<Vec<String>>> {
        let doc = self.http_get(&format!("/contests/{}", contest_id))?;

        Html::parse_document(&doc)
            .select(&Selector::parse("#contest-statement > .lang > .lang-ja table").unwrap())
            .filter(|table| {
                let header = table
                    .select(&Selector::parse("thead > tr > th").unwrap())
                    .flat_map(|r| r.text())
                    .collect::<Vec<_>>();
                header == ["Task", "Score"] || header == ["問題", "点数"]
            })
            .exactly_one()
            .ok()
            .map(|table| {
                table
                    .select(&Selector::parse("tbody > tr").unwrap())
                    .map(|tr| {
                        let text = tr
                            .select(&Selector::parse("td").unwrap())
                            .flat_map(|r| r.text())
                            .collect::<Vec<_>>();
                        match text.len() {
                            2 => Ok(text[0].to_owned()),
                            _ => Err(anyhow!("could not parse the table")),
                        }
                    })
                    .collect()
            })
            .transpose()
    }

    pub fn contest_info(&self, contest_id: &str) -> Result<ContestInfo> {
        self.check_login()?;

        let doc = self.http_get(&format!("/contests/{}/tasks", contest_id))?;

        let doc = Html::parse_document(&doc);
        let sel_problem = Selector::parse("table tbody tr").unwrap();

        let mut problems = vec![];

        for row in doc.select(&sel_problem) {
            let sel_td = Selector::parse("td").unwrap();
            let mut it = row.select(&sel_td);
            let c1 = it.next().unwrap();
            let c2 = it.next().unwrap();
            let c3 = it.next().unwrap();
            let c4 = it.next().unwrap();

            let id = c1
                .select(&Selector::parse("a").unwrap())
                .next()
                .unwrap()
                .inner_html();

            let name = c2
                .select(&Selector::parse("a").unwrap())
                .next()
                .unwrap()
                .inner_html();

            let url = c2
                .select(&Selector::parse("a").unwrap())
                .next()
                .unwrap()
                .value()
                .attr("href")
                .unwrap();

            let tle = c3.inner_html();
            let mle = c4.inner_html();

            problems.push(Problem {
                id: id.trim().to_owned(),
                name: name.trim().to_owned(),
                url: url.trim().to_owned(),
                tle: tle.trim().to_owned(),
                mle: mle.trim().to_owned(),
            });
        }

        Ok(ContestInfo { problems })
    }

    pub fn test_cases(&self, problem_url: &str) -> Result<Vec<TestCase>> {
        self.check_login()?;

        let doc = self.http_get(problem_url)?;

        let doc = Html::parse_document(&doc);

        let h3_sel = Selector::parse("h3").unwrap();

        let mut inputs_ja = vec![];
        let mut outputs_ja = vec![];
        let mut inputs_en = vec![];
        let mut outputs_en = vec![];

        for r in doc.select(&h3_sel) {
            let p = ElementRef::wrap(r.parent().unwrap()).unwrap();
            let label = p.select(&h3_sel).next().unwrap().inner_html();
            let label = label.trim();
            // dbg!(r.parent().unwrap().first_child().unwrap().value());

            // let label = r
            //     .prev_sibling()
            //     .unwrap()
            //     .first_child()
            //     .unwrap()
            //     .value()
            //     .as_text()
            //     .unwrap();

            let f = || {
                p.select(&Selector::parse("pre").unwrap())
                    .next()
                    .unwrap()
                    .inner_html()
                    .trim()
                    .to_owned()
            };
            if label.starts_with("入力例") {
                inputs_ja.push(f());
            }
            if label.starts_with("出力例") {
                outputs_ja.push(f());
            }

            if label.starts_with("Sample Input") {
                inputs_en.push(f());
            }
            if label.starts_with("Sample Output") {
                outputs_en.push(f());
            }
        }

        assert_eq!(inputs_ja.len(), outputs_ja.len());
        assert_eq!(inputs_en.len(), outputs_en.len());

        let (inputs, outputs) = if inputs_ja.len() >= inputs_en.len() {
            (inputs_ja, outputs_ja)
        } else {
            (inputs_en, outputs_en)
        };

        let mut ret = vec![];
        for i in 0..inputs.len() {
            ret.push(TestCase {
                input: inputs[i].clone(),
                output: outputs[i].clone(),
            });
        }
        Ok(ret)
    }

    pub fn submit(&self, contest_id: &str, problem_id: &str, source_code: &str) -> Result<()> {
        self.check_login()?;

        let doc = self.http_get(&format!("/contests/{}/submit", contest_id))?;

        let (task_screen_name, language_id, language_name, csrf_token) = {
            let doc = Html::parse_document(&doc);

            let task_screen_name = (|| {
                for r in doc.select(
                    &Selector::parse("select[name=\"data.TaskScreenName\"] option").unwrap(),
                ) {
                    if r.inner_html()
                        .trim()
                        .split_whitespace()
                        .next()
                        .unwrap()
                        .to_lowercase()
                        .starts_with(&problem_id.to_lowercase())
                    {
                        return Ok(r.value().attr("value").unwrap());
                    }
                }
                Err(anyhow!("Problem not found: {}", problem_id))
            })()?;

            let (language_id, language_name) = (|| {
                for r in doc.select(
                    &Selector::parse(&format!(
                        "div[id=\"select-lang-{}\"] select option",
                        &task_screen_name
                    ))
                    .unwrap(),
                ) {
                    if r.inner_html()
                        .trim()
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_lowercase()
                        .starts_with("rust")
                    {
                        return Ok((r.value().attr("value").unwrap(), r.inner_html()));
                    }
                }
                Err(anyhow!(
                    "Rust seems to be not available in problem {}...",
                    problem_id
                ))
            })()?;

            let csrf_token = doc
                .select(&Selector::parse("input[name=\"csrf_token\"]").unwrap())
                .next()
                .unwrap()
                .value()
                .attr("value")
                .unwrap();

            (
                task_screen_name.to_owned(),
                language_id.to_owned(),
                language_name,
                csrf_token.to_owned(),
            )
        };

        let _ = self.http_post_form(
            &format!("/contests/{}/submit", contest_id),
            &[
                ("data.TaskScreenName", &task_screen_name),
                ("data.LanguageId", &language_id),
                ("sourceCode", &source_code),
                ("csrf_token", &csrf_token),
            ],
        )?;

        println!(
            "Submitted to problem `{}`, using language `{}`",
            task_screen_name, language_name
        );
        Ok(())
    }

    pub fn submission_status(&self, contest_id: &str) -> Result<Vec<SubmissionResult>> {
        self.check_login()?;

        // FIXME: Currently, this returns only up to 20 submissions

        let con = self.http_get(&format!("/contests/{}/submissions/me", contest_id))?;
        let doc = Html::parse_document(&con);

        let mut ret = vec![];

        for r in doc.select(&Selector::parse("table tbody tr").unwrap()) {
            // <td class="no-break"><time class='fixtime fixtime-second'>2020-01-18 03:59:59+0900</time></td>
            // <td><a href="/contests/abc123/tasks/abc123_a">A - Five Antennas</a></td>
            // <td><a href="/users/tanakh">tanakh</a> <a href='/contests/abc123/submissions?f.User=tanakh'><span class='glyphicon glyphicon-search black' aria-hidden='true' data-toggle='tooltip' title='tanakhさんの提出を見る'></span></a></td>
            // <td>Rust (1.15.1)</td>
            // <td class="text-right submission-score" data-id="9551881">0</td>
            // <td class="text-right">1970 Byte</td>
            // <td class='text-center'><span class='label label-warning' aria-hidden='true' data-toggle='tooltip' data-placement='top' title="実行時間制限超過">TLE</span>
            // </td>
            // <td class='text-right'>2103 ms</td>
            // <td class='text-right'>4352 KB</td>
            // <td class="text-center">
            //     <a href="/contests/abc123/submissions/9551881">詳細</a>
            // </td>

            let res = (|| -> Option<SubmissionResult> {
                let sel = Selector::parse("td").unwrap();
                let mut it = r.select(&sel);

                let date = it.next()?.first_child()?.first_child()?.value().as_text()?;
                let date = chrono::DateTime::parse_from_str(date, "%Y-%m-%d %H:%M:%S%z")
                    .ok()?
                    .into();
                let problem_name = it
                    .next()?
                    .first_child()?
                    .first_child()?
                    .value()
                    .as_text()?
                    .to_string();
                let user = it
                    .next()?
                    .first_child()?
                    .first_child()?
                    .value()
                    .as_text()?
                    .to_string();
                let language = it.next()?.first_child()?.value().as_text()?.to_string();
                let t = it.next()?;
                let id: usize = t.value().attr("data-id")?.parse().ok()?;
                let score: i64 = t.first_child()?.value().as_text()?.parse().ok()?;
                let code_length = it.next()?.first_child()?.value().as_text()?.to_string();
                let status = StatusCode::from_str(
                    it.next()?.first_child()?.first_child()?.value().as_text()?,
                )?;

                let resource = (|| {
                    let run_time = it.next()?.first_child()?.value().as_text()?.to_string();
                    let memory = it.next()?.first_child()?.value().as_text()?.to_string();
                    Some((run_time, memory))
                })();

                Some(SubmissionResult {
                    id,
                    date,
                    problem_name,
                    user,
                    language,
                    score,
                    code_length,
                    status,
                    run_time: resource.as_ref().map(|r| r.0.clone()),
                    memory: resource.map(|r| r.1),
                })
            })();

            if res.is_none() {
                panic!("failed to parse result:\n{}", r.html());
            }

            ret.push(res.unwrap());
        }

        Ok(ret)
    }

    pub fn submission_status_full(
        &self,
        contest_id: &str,
        submission_id: usize,
    ) -> Result<FullSubmissionResult> {
        self.check_login()?;
        let con = self.http_get(&format!(
            "/contests/{}/submissions/{}",
            contest_id, submission_id,
        ))?;
        let doc = Html::parse_document(&con);

        // <table class="table table-bordered table-striped">
        // <tr>
        //     <th class="col-sm-4">提出日時</th>
        //     <td class="text-center"><time class='fixtime fixtime-second'>2020-01-19 21:53:37+0900</time></td>
        // </tr>
        // <tr>
        //     <th>問題</th>
        //     <td class="text-center"><a href='/contests/abc152/tasks/abc152_f'>F - Tree and Constraints</a></td>
        // </tr>
        // <tr>
        //     <th>ユーザ</th>
        //     <td class="text-center"><a href='/users/tanakh'>tanakh</a> <a href='/contests/abc152/submissions?f.User=tanakh'><span class='glyphicon glyphicon-search black' aria-hidden='true' data-toggle='tooltip' title='tanakhさんの提出を見る'></span></a></td>
        // </tr>
        // <tr>
        //     <th>言語</th>
        //     <td class="text-center">Rust (1.15.1)</td>
        // </tr>
        // <tr>
        //     <th>得点</th>
        //     <td class="text-center">0</td>
        // </tr>
        // <tr>
        //     <th>コード長</th>
        //     <td class="text-center">321502 Byte</td>
        // </tr>
        // <tr>
        //     <th>結果</th>
        //     <td id="judge-status" class="text-center"><span class='label label-warning' aria-hidden='true' data-toggle='tooltip' data-placement='top' title="不正解">WA</span></td>
        // </tr>
        //     <tr>
        //         <th>実行時間</th>
        //         <td class="text-center">4215 ms</td>
        //     </tr>
        //     <tr>
        //         <th>メモリ</th>
        //         <td class="text-center">8828 KB</td>
        //     </tr>
        // </table>

        let result = (|| -> Option<SubmissionResult> {
            let sel = Selector::parse("table tr th+td").unwrap();

            let mut it = doc.select(&sel);

            let date = it.next()?.first_child()?.first_child()?.value().as_text()?;
            let date = chrono::DateTime::parse_from_str(date.trim(), "%Y-%m-%d %H:%M:%S%z")
                .ok()?
                .into();
            let problem_name = it
                .next()?
                .first_child()?
                .first_child()?
                .value()
                .as_text()?
                .trim()
                .to_owned();
            let user = it.next()?.inner_html().trim().to_owned();
            let language = it.next()?.inner_html().trim().to_owned();
            let score: i64 = it.next()?.inner_html().trim().to_owned().parse().ok()?;
            let code_length = it.next()?.inner_html().trim().to_owned();
            let status =
                StatusCode::from_str(it.next()?.first_child()?.first_child()?.value().as_text()?)?;

            let resource = (|| {
                let run_time = it.next()?.first_child()?.value().as_text()?.to_string();
                let memory = it.next()?.first_child()?.value().as_text()?.to_string();
                Some((run_time, memory))
            })();

            Some(SubmissionResult {
                id: submission_id,
                date,
                problem_name,
                user,
                language,
                score,
                code_length,
                status,
                run_time: resource.as_ref().map(|r| r.0.clone()),
                memory: resource.map(|r| r.1),
            })
        })()
        .with_context(|| "Failed to parse result")?;

        // <table class="table table-bordered table-striped th-center">
        // <thead>
        // <tr>
        //     <th>ケース名</th>
        //     <th>結果</th>
        //     <th>実行時間</th>
        //     <th>メモリ</th>
        // </tr>
        // </thead>
        // <tbody>
        // <tr>
        //     <td class="text-center">dense_01.txt</td>
        //         <td class="text-center"><span class='label label-success' aria-hidden='true' data-toggle='tooltip' data-placement='top' title="正解">AC</span></td>
        //         <td class="text-right">705 ms</td>
        //         <td class="text-right">8824 KB</td>

        // </tr>

        let sel_td = Selector::parse("td").unwrap();

        let mut cases = vec![];

        for r in doc.select(&Selector::parse("table tbody tr").unwrap()) {
            let case = (|| -> Option<CaseResult> {
                let mut it = r.select(&sel_td);
                let name = it.next()?.inner_html();
                let result = StatusCode::from_str(
                    it.next()?.first_child()?.first_child()?.value().as_text()?,
                )?;
                let run_time = it.next()?.inner_html();
                let memory = it.next()?.inner_html();

                Some(CaseResult {
                    name,
                    result,
                    run_time: Some(run_time),
                    memory: Some(memory),
                })
            })();

            if let Some(case) = case {
                cases.push(case);
            }
        }

        let ret = FullSubmissionResult { result, cases };

        Ok(ret)
    }

    fn http_get(&self, path: &str) -> anyhow::Result<String> {
        self.http_req("GET", path, Request::call)
    }

    fn http_post_form(&self, path: &str, form: &[(&str, &str)]) -> anyhow::Result<String> {
        self.http_req("POST", path, |r| r.send_form(form))
    }

    fn http_req(
        &self,
        mut method: &str,
        path: &str,
        call: impl FnOnce(&mut Request) -> Response,
    ) -> anyhow::Result<String> {
        let mut url = "https://atcoder.jp".parse::<Url>().unwrap();
        url.set_path(path);

        let mut call = Box::new(call) as Box<dyn FnOnce(&mut _) -> _>;

        // Redirects manually.
        let mut rest_redirects = REDIRECTS;
        return loop {
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
            url = url.join(location)?;
            method = "GET";

            ensure!(url.host_str() == Some("atcoder.jp"), "cross host");

            rest_redirects -= 1;
        };

        const REDIRECTS: u32 = 5;
    }
}

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
