use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use itertools::Itertools as _;
use regex::Regex;
use scraper::{Html, Selector};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const ATCODER_ENDPOINT: &str = "https://atcoder.jp";

pub struct AtCoder {
    client: reqwest::Client,
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

async fn http_get(client: &reqwest::Client, url: &str) -> Result<String> {
    Ok(client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

impl Drop for AtCoder {
    fn drop(&mut self) {
        let v = self.client.cookie_store_json().unwrap();
        let dir = self.session_file.parent().unwrap();
        fs::create_dir_all(dir).unwrap();
        fs::write(&self.session_file, v).unwrap();
    }
}

impl AtCoder {
    pub fn new(session_file: &Path) -> Result<AtCoder> {
        let cb = reqwest::Client::builder().cookie_store(true);

        let cb = match fs::read(session_file) {
            Ok(v) => cb.set_cookie_store(v),
            Err(err) => {
                if err.kind() == ErrorKind::NotFound {
                    cb
                } else {
                    return Err(err.into());
                }
            }
        };

        Ok(AtCoder {
            client: cb.build()?,
            session_file: session_file.to_owned(),
        })
    }

    async fn check_login(&self) -> Result<()> {
        let _ = self
            .username()
            .await?
            .ok_or_else(|| anyhow!("You are not logged in. Please login first."))?;
        Ok(())
    }

    pub async fn username(&self) -> Result<Option<String>> {
        let doc = http_get(&self.client, ATCODER_ENDPOINT).await?;
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

    pub async fn login(&self, username: &str, password: &str) -> Result<()> {
        let doc = http_get(&self.client, &format!("{}/login", ATCODER_ENDPOINT)).await?;

        let document = Html::parse_document(&doc);

        let csrf_token = document
            .select(&Selector::parse("input[name=\"csrf_token\"]").unwrap())
            .next()
            .ok_or_else(|| anyhow!("cannot find csrf_token"))?;

        let csrf_token = csrf_token
            .value()
            .attr("value")
            .ok_or_else(|| anyhow!("cannot find csrf_token"))?;

        let res = self
            .client
            .post(&format!("{}/login", ATCODER_ENDPOINT))
            .form(&[
                ("username", username),
                ("password", password),
                ("csrf_token", csrf_token),
            ])
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

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
            return Err(anyhow!(
                "Login failed: {}",
                err.last_child().unwrap().value().as_text().unwrap().trim()
            ));
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

    pub async fn problem_ids_of_rated_contest(
        &self,
        contest_id: &str,
    ) -> Result<Option<Vec<String>>> {
        let t = format!("{}/contests/{}", ATCODER_ENDPOINT, contest_id);
        let doc = http_get(&self.client, &t).await?;

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

    pub async fn contest_info(&self, contest_id: &str) -> Result<ContestInfo> {
        self.check_login().await?;

        let t = format!("{}/contests/{}/tasks", ATCODER_ENDPOINT, contest_id);
        let doc = http_get(&self.client, &t).await?;

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

    pub async fn test_cases(&self, problem_url: &str) -> Result<Vec<TestCase>> {
        self.check_login().await?;

        let t = format!("{}{}", ATCODER_ENDPOINT, problem_url);
        let doc = http_get(&self.client, &t).await?;

        let doc = Html::parse_document(&doc);

        let mut inputs = vec![];
        let mut outputs = vec![];

        for r in doc.select(&Selector::parse("h3+pre").unwrap()) {
            let label = r
                .prev_sibling()
                .unwrap()
                .first_child()
                .unwrap()
                .value()
                .as_text()
                .unwrap();

            if label.starts_with("Sample Input") {
                inputs.push(r.inner_html().trim().to_owned());
            }
            if label.starts_with("Sample Output") {
                outputs.push(r.inner_html().trim().to_owned());
            }
        }

        assert_eq!(inputs.len(), outputs.len());

        let mut ret = vec![];
        for i in 0..inputs.len() {
            ret.push(TestCase {
                input: inputs[i].clone(),
                output: outputs[i].clone(),
            });
        }
        Ok(ret)
    }

    pub async fn submit(
        &self,
        contest_id: &str,
        problem_id: &str,
        source_code: &str,
    ) -> Result<()> {
        self.check_login().await?;

        let t = format!("{}/contests/{}/submit", ATCODER_ENDPOINT, contest_id);
        let doc = http_get(&self.client, &t).await?;

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
                        .unwrap()
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

        let t = format!("{}/contests/{}/submit", ATCODER_ENDPOINT, contest_id);
        let _res = self
            .client
            .post(&t)
            .form(&[
                ("data.TaskScreenName", &task_screen_name),
                ("data.LanguageId", &language_id),
                ("sourceCode", &source_code.to_owned()),
                ("csrf_token", &csrf_token),
            ])
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        println!(
            "Submitted to problem `{}`, using language `{}`",
            task_screen_name, language_name
        );
        Ok(())
    }

    pub async fn submission_status(&self, contest_id: &str) -> Result<Vec<SubmissionResult>> {
        self.check_login().await?;

        // FIXME: Currently, this returns only up to 20 submissions

        let t = format!(
            "{}/contests/{}/submissions/me",
            ATCODER_ENDPOINT, contest_id
        );
        let con = http_get(&self.client, &t).await?;
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

    pub async fn submission_status_full(
        &self,
        contest_id: &str,
        submission_id: usize,
    ) -> Result<FullSubmissionResult> {
        self.check_login().await?;
        let t = format!(
            "{}/contests/{}/submissions/{}",
            ATCODER_ENDPOINT, contest_id, submission_id,
        );
        let con = http_get(&self.client, &t).await?;
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
        .ok_or_else(|| anyhow!("Failed to parse result"))?;

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
}
