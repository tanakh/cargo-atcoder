use anyhow::{anyhow, Result};
use scraper::{Html, Selector};
use serde_derive::Deserialize;
use std::collections::BTreeMap;

const ATCODER_ENDPOINT: &str = "https://atcoder.jp";

pub struct AtCoder {
    client: reqwest::Client,
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
}

#[derive(Debug, Deserialize)]
pub struct SubmissionResults {
    #[serde(rename = "Result")]
    pub result: BTreeMap<String, SubmissionResult>,
}

#[derive(Debug, Deserialize)]
pub struct SubmissionResult {
    #[serde(rename = "Html")]
    html: String,
    #[serde(rename = "Score")]
    score: String,
}

#[derive(Debug)]
pub struct ResultStatus {
    pub status: String,
    pub time: Option<String>,
    pub mem: Option<String>,
}

enum ResultType {
    Waiting(String),
    Progress(String),
    Done(String),
}

impl SubmissionResult {
    pub fn status(&self) -> ResultStatus {
        let doc = Html::parse_fragment(&format!("<table><tr>{}</tr></table>", self.html));

        let status = doc
            .select(&Selector::parse("td span").unwrap())
            .next()
            .unwrap()
            .inner_html()
            .trim()
            .to_owned();

        let sel = Selector::parse("td").unwrap();
        let mut it = doc.select(&sel);
        let _ = it.next();
        let time = it.next().map(|r| r.inner_html().trim().to_owned());
        let mem = it.next().map(|r| r.inner_html().trim().to_owned());

        ResultStatus { status, time, mem }
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

impl AtCoder {
    pub fn new() -> Result<AtCoder> {
        Ok(AtCoder {
            client: reqwest::Client::builder().cookie_store(true).build()?,
        })
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<()> {
        let doc = http_get(&self.client, &format!("{}/login", ATCODER_ENDPOINT)).await?;

        let document = Html::parse_document(&doc);

        // dbg!(&document);

        let csrf_token = document
            .select(&Selector::parse("input[name=\"csrf_token\"]").unwrap())
            .next()
            .ok_or(anyhow!("cannot find csrf_token"))?;

        // dbg!(&csrf_token);

        let csrf_token = csrf_token
            .value()
            .attr("value")
            .ok_or(anyhow!("cannot find csrf_token"))?;

        let res = self
            .client
            .post(&format!("{}/login", ATCODER_ENDPOINT))
            .form(&[
                ("username", username.as_ref()),
                ("password", password.as_ref()),
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
        //   {error message}
        // </div>
        if let Some(err) = res
            .select(&Selector::parse("div.alert-danger").unwrap())
            .next()
        {
            Err(anyhow!(
                "Login failed: {}",
                err.last_child().unwrap().value().as_text().unwrap().trim()
            ))?
        }

        // On success:
        // <div class="alert alert-success alert-dismissible col-sm-12 fade in" role="alert" >
        //     ...
        //     ようこそ、tanakh さん。
        // </div>
        if let Some(_) = res
            .select(&Selector::parse("div.alert-success").unwrap())
            .next()
        {
            return Ok(());
        }

        Err(anyhow!("Login failed: Unknown error"))
    }

    pub async fn contest_info(&self, contest_id: &str) -> Result<ContestInfo> {
        let doc = http_get(
            &self.client,
            &format!("{}/contests/{}/tasks", ATCODER_ENDPOINT, contest_id),
        )
        .await?;

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

            // dbg!(&id, &name, &url, &tle, &mle);
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
        let doc = http_get(
            &self.client,
            &format!("{}{}", ATCODER_ENDPOINT, problem_url),
        )
        .await?;
        // eprintln!("{}", doc);
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
        let doc = http_get(
            &self.client,
            &format!("{}/contests/{}/submit", ATCODER_ENDPOINT, contest_id),
        )
        .await?;
        let doc = Html::parse_document(&doc);

        let task_screen_name = (|| {
            for r in
                doc.select(&Selector::parse("select[name=\"data.TaskScreenName\"] option").unwrap())
            {
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

        dbg!(&task_screen_name);

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

        // dbg!(language_id, language_name, csrf_token);

        println!(
            "Submit to problem={}, using language={}",
            task_screen_name, language_name
        );

        let _res = self
            .client
            .post(&format!(
                "{}/contests/{}/submit",
                ATCODER_ENDPOINT, contest_id
            ))
            .form(&[
                ("data.TaskScreenName", task_screen_name),
                ("data.LanguageId", language_id),
                ("sourceCode", source_code),
                ("csrf_token", csrf_token),
            ])
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        Ok(())
    }

    pub async fn submission_status(&self, contest_id: &str) -> Result<SubmissionResults> {
        let con = http_get(
            &self.client,
            &format!(
                "{}/contests/{}/submissions/me/status/json",
                ATCODER_ENDPOINT, contest_id
            ),
        )
        .await?;

        Ok(serde_json::from_str(&con)?)
    }
}
