use std::{
    fmt::Display,
    fs::{self, OpenOptions},
    io::Write,
    iter::empty,
    ops::Range,
    path::{Path, PathBuf},
};

use indexmap::IndexSet;
use jaq_core::{
    Compiler, Ctx, Native, RcIter, compile,
    load::{self, Arena, File, Loader},
};
use jaq_json::Val;
use serde_json::Value;
use tracing::{info, warn};

use crate::{
    config::AppDirs,
    error::{Error, Result},
};

const INITIAL_PROMPT: &str = ".";
const TOPIC_HISTORY_DELIMITER: &str = ":|:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JqError {
    pub message: String,
    pub span: Range<usize>,
}

impl JqError {
    pub fn display(&self) -> String {
        format!("{} ({:?})", self.message, self.span)
    }

    fn compile(code: &str, (found, undefined): compile::Error<&str>) -> Self {
        use compile::Undefined::Filter;
        let wnoa = |exp, got| format!("wrong number of arguments (expected {exp}, found {got})");
        Self {
            message: match (found, undefined) {
                ("reduce", Filter(arity)) => wnoa("2", arity),
                ("foreach", Filter(arity)) => wnoa("2 or 3", arity),
                (_, undefined) => format!("undefined {}", undefined.as_str()),
            },
            span: load::span(code, found),
        }
    }

    fn io(code: &str, (path, error): (&str, String)) -> Self {
        Self {
            message: format!("could not load file {path}: {error}"),
            span: load::span(code, path),
        }
    }

    fn lexer(code: &str, (expected, found): load::lex::Error<&str>) -> Self {
        let span = load::span(code, found);
        let unexpected = match &found[..found.char_indices().nth(1).map_or(found.len(), |(i, _)| i)]
        {
            "" => "end of input",
            _ => "character",
        };

        let expected = match &expected {
            load::lex::Expect::Delim(_) => "unclosed delimiter",
            _ => expected.as_str(),
        };

        Self {
            message: format!("unexpected {unexpected}, expected {expected}"),
            span,
        }
    }

    fn parse(code: &str, (expected, found): load::parse::Error<&str>) -> Self {
        let span = load::span(code, found);
        let unexpected = if found.is_empty() {
            "end of input"
        } else {
            "token"
        };
        Self {
            message: format!("unexpected {unexpected}, expected {}", expected.as_str()),
            span,
        }
    }

    fn runtime(code: &str, error: impl Display) -> Self {
        Self {
            message: error.to_string(),
            span: 0..code.len(),
        }
    }
}

pub struct Jq;

impl Jq {
    pub fn check(code: &str) -> std::result::Result<(), Vec<JqError>> {
        compile_filter(code).map(|_| ())
    }

    pub fn run(code: &str, value: &Value) -> std::result::Result<Vec<Value>, Vec<JqError>> {
        let filter = compile_filter(code)?;
        let input = RcIter::new(empty());
        let mut values = Vec::new();
        let mut errors = Vec::new();
        for item in filter.run((Ctx::new([], &input), Val::from(value.clone()))) {
            match item {
                Ok(val) => values.push(Value::from(val)),
                Err(error) => errors.push(JqError::runtime(code, error)),
            }
        }
        if errors.is_empty() {
            Ok(values)
        } else {
            Err(errors)
        }
    }
}

fn compile_filter(code: &str) -> std::result::Result<jaq_core::Filter<Native<Val>>, Vec<JqError>> {
    let loader = Loader::new(jaq_std::defs().chain(jaq_json::defs()));
    let arena = Arena::default();
    let program = File { code, path: () };
    let modules = loader.load(&arena, program).map_err(|e| {
        e.into_iter()
            .flat_map(|(file, err)| match err {
                jaq_core::load::Error::Io(errors) => errors
                    .into_iter()
                    .map(|error| JqError::io(file.code, error))
                    .collect::<Vec<_>>(),
                jaq_core::load::Error::Lex(errors) => errors
                    .into_iter()
                    .map(|error| JqError::lexer(file.code, error))
                    .collect::<Vec<_>>(),
                jaq_core::load::Error::Parse(errors) => errors
                    .into_iter()
                    .map(|error| JqError::parse(file.code, error))
                    .collect::<Vec<_>>(),
            })
            .collect::<Vec<_>>()
    })?;

    Compiler::default()
        .with_funs(jaq_std::funs().chain(jaq_json::funs()))
        .compile(modules)
        .map_err(|e| {
            e.into_iter()
                .flat_map(|(file, err)| err.into_iter().map(|e| JqError::compile(file.code, e)))
                .collect::<Vec<_>>()
        })
}

/// A history item composed of the JQ filter and the topic suffix it was used on.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
struct Item {
    topic: String,
    filter: String,
}

impl Display for Item {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{TOPIC_HISTORY_DELIMITER}{}", self.topic, self.filter)
    }
}

impl Item {
    fn new(topic: &str, filter: &str) -> Self {
        Self {
            topic: topic.split('/').next_back().unwrap_or(topic).into(),
            filter: filter.into(),
        }
    }

    fn matches(&self, topic: &str) -> bool {
        let suffix = topic.split('/').next_back().unwrap_or(topic);
        self.topic.is_empty() || self.topic.ends_with(suffix)
    }
}

pub struct JqHistory {
    path: PathBuf,
    committed: IndexSet<Item>,
    staging: String,
}

impl JqHistory {
    pub fn load() -> Result<Self> {
        let dirs = AppDirs::new()?;
        crate::config::migrate_legacy(&dirs)?;
        Self::load_from(dirs.jq_history_file())
    }

    pub fn load_from(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            fs::File::create(&path)?;
        }
        let content = fs::read_to_string(&path)
            .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
        let list = content
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| {
                let (topic, filter) = line
                    .split_once(TOPIC_HISTORY_DELIMITER)
                    .unwrap_or(("", line));
                Item::new(topic, filter)
            })
            .rev()
            .collect::<IndexSet<_>>();

        tracing::debug!(amount = list.len(), "loading JQ history");
        Ok(Self {
            path,
            committed: list,
            staging: Default::default(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn stage(&mut self, prompt: &str) {
        tracing::debug!("stage jq filter");
        self.staging = prompt.into();
    }

    pub fn commit(&mut self, topic: &str) {
        if self.staging.is_empty() || self.staging == INITIAL_PROMPT {
            return;
        }
        let commit = self.staging.clone();
        let topic = topic.split('/').next_back().unwrap_or(topic);
        info!("commit jq history");
        let item = Item::new(topic, &commit);
        if !self.committed.shift_insert(0, item.clone()) {
            return;
        }

        let result = OpenOptions::new()
            .append(true)
            .open(&self.path)
            .map_err(Error::from)
            .and_then(|mut file| writeln!(file, "{item}").map_err(Error::from));
        if let Err(e) = result {
            warn!(error = %e, "could not save jq history");
        }
    }

    pub fn list(&self, topic: &str) -> Vec<String> {
        self.committed
            .iter()
            .filter(|item| item.matches(topic))
            .map(|item| item.filter.clone())
            .collect()
    }

    pub fn len(&self, topic: &str) -> usize {
        self.matching(topic).count()
    }

    pub fn is_empty(&self, topic: &str) -> bool {
        self.len(topic) == 0
    }

    /// Lookup from the bottom of history. Index 0 is the staged prompt.
    pub fn lookup(&self, index: usize, topic: &str) -> Option<String> {
        if self.is_empty(topic) {
            return None;
        }
        if index == 0 {
            info!(index, "lookup jq history");
            return Some(self.staging.clone());
        }

        let commit = self.matching(topic).nth(index - 1)?.filter.as_str();
        info!(index, "lookup jq history");
        Some(commit.into())
    }

    fn matching(&self, topic: &str) -> impl Iterator<Item = &Item> {
        self.committed
            .iter()
            .filter(|item| item.filter.starts_with(&self.staging))
            .filter(|item| item.matches(topic))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_identity_and_field() {
        let value = serde_json::json!({"bri": 180});
        let out = Jq::run(".", &value).unwrap();
        assert_eq!(out, vec![value.clone()]);
        let out = Jq::run(".bri", &value).unwrap();
        assert_eq!(out, vec![serde_json::json!(180)]);
    }

    #[test]
    fn compile_error_has_span() {
        let errors = Jq::check("[").unwrap_err();
        assert!(!errors.is_empty());
        assert!(!errors[0].message.is_empty());
    }

    #[test]
    fn runtime_error_is_returned() {
        let value = serde_json::json!(1);
        let errors = Jq::run(".[]", &value).unwrap_err();
        assert!(!errors.is_empty());
        assert!(!errors[0].message.is_empty());
    }

    #[test]
    fn history_groups_by_topic_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jq");
        let mut history = JqHistory::load_from(&path).unwrap();
        history.stage(".bri");
        history.commit("home/living/lamp");
        history.stage(".temp");
        history.commit("home/kitchen/fridge");

        assert_eq!(history.list("office/lamp"), vec![".bri".to_string()]);
        assert_eq!(history.list("fridge"), vec![".temp".to_string()]);
        assert!(path.exists());
    }
}
