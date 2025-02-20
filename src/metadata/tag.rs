use crate::index::document_repository::DocumentData;
use crate::index::DocId;
use regex::{Regex, RegexBuilder};
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use whatlang::detect;

//Error Handling
use thiserror::Error;
#[derive(Error, Debug)]
pub enum TaggingError {
    #[error("cannot compile regex expression")]
    RegexError(#[from] regex::Error),
    #[error("could not load/write configuration file")]
    ConfigError(#[from] confy::ConfyError),
    #[error("body of document {0} is empty")]
    EmptyBody(DocId),
}

pub trait Matcher {
    fn match_document(&self, doc: &DocumentData) -> Result<bool, TaggingError>;
}

///////////////////////////////////// NoMatcher ///////////////////////////////////////
/// Matcher matching no document
pub struct NoMatcher {}

impl NoMatcher {
    pub fn new() -> NoMatcher {
        NoMatcher {}
    }
}

impl Matcher for NoMatcher {
    fn match_document(&self, _: &DocumentData) -> Result<bool, TaggingError> {
        Ok(false)
    }
}
///////////////////////////////////// Regex Matcher ///////////////////////////////////////
pub struct RegexMatcher {
    regex: Regex,
}

impl RegexMatcher {
    pub fn new(regex: Regex) -> RegexMatcher {
        RegexMatcher { regex }
    }

    pub fn parse_string(regex_str: &str) -> Result<RegexMatcher, TaggingError> {
        Ok(RegexMatcher::new(Regex::new(regex_str)?))
    }
}

impl Matcher for RegexMatcher {
    fn match_document(&self, doc: &DocumentData) -> Result<bool, TaggingError> {
        let body = match &doc.body {
            Some(b) => b,
            None => {
                return Err(TaggingError::EmptyBody(doc.id));
            }
        };
        Ok(self.regex.is_match(body))
    }
}

///////////////////////////////////// Full Matcher ///////////////////////////////////////

pub struct FullMatcher {
    regex: RegexMatcher,
}

impl FullMatcher {
    pub fn new(term: &str, case_insensitive: bool) -> Result<FullMatcher, TaggingError> {
        Ok(FullMatcher {
            regex: RegexMatcher::new(
                RegexBuilder::new(term)
                    .case_insensitive(case_insensitive)
                    .build()?,
            ),
        })
    }
}

impl Matcher for FullMatcher {
    fn match_document(&self, doc: &DocumentData) -> Result<bool, TaggingError> {
        self.regex.match_document(doc)
    }
}

///////////////////////////////////// Any Matcher ///////////////////////////////////////

pub struct AnyMatcher {
    regex: RegexMatcher,
}

impl AnyMatcher {
    pub fn new(term: &str, case_insensitive: bool) -> Result<AnyMatcher, TaggingError> {
        let terms = "(".to_owned()
            + &term
                .split(',')
                .map(|s| s.trim())
                .collect::<Vec<&str>>()
                .join("|")
            + ")";
        Ok(AnyMatcher {
            regex: RegexMatcher::new(
                RegexBuilder::new(&terms)
                    .case_insensitive(case_insensitive)
                    .build()?,
            ),
        })
    }
}

impl Matcher for AnyMatcher {
    fn match_document(&self, doc: &DocumentData) -> Result<bool, TaggingError> {
        self.regex.match_document(doc)
    }
}

///////////////////////////////////// Tagger ///////////////////////////////////////
///
pub struct Tagger {
    tags: HashMap<TagId, TagConfig>,
    tags_file: PathBuf,
    extract_extended_metadata: bool,
}

impl Tagger {
    pub fn new(data_dir: &Path, extract_extended_metadata: bool) -> Result<Tagger, TaggingError> {
        let mut tagger = Tagger {
            tags: HashMap::new(),
            tags_file: data_dir.join("tags.toml"),
            extract_extended_metadata,
        };
        tagger.load_config()?;
        Ok(tagger)
    }

    pub fn add_tag(&mut self, mut tag: TagConfig) -> Result<(), TaggingError> {
        //Load config
        let mut cfg: TagsConfig = confy::load_path(&self.tags_file)?;

        cfg.curr_id += 1;
        tag.id = cfg.curr_id;

        //Store config
        debug!("Storing tag {:#?} in {:?}", &tag, self.tags_file);
        cfg.tags.push(tag.clone());
        confy::store_path(&self.tags_file, cfg)?;

        //Add to hashmap
        self.tags.insert(tag.id, tag);

        Ok(())
    }

    pub fn add_or_replace_tag(&mut self, mut tag: TagConfig) -> Result<(), TaggingError> {
        //Load config
        let mut cfg: TagsConfig = confy::load_path(&self.tags_file)?;

        if !self.tags.contains_key(&tag.id) {
            //Set new ID
            cfg.curr_id += 1;
            tag.id = cfg.curr_id;
            info!("Adding new tag with id {} and name `{}` ", tag.id, tag.name);
        } else {
            info!("Replacing tag {} with new tag", tag.id);
            //Remove from cfg, as it will be readded later
            cfg.tags.retain(|t| t.id != tag.id);
        }

        //Store config
        debug!("Storing tag {:#?} in {:?}", &tag, self.tags_file);
        cfg.tags.push(tag.clone());
        confy::store_path(&self.tags_file, cfg)?;

        //Add to hashmap
        self.tags.insert(tag.id, tag);

        Ok(())
    }

    pub fn remove_tag(&mut self, id: TagId) -> Result<(), TaggingError> {
        info!("Remove tag {} from tag repository", id);
        //Change in  config
        let mut cfg: TagsConfig = confy::load_path(&self.tags_file)?;
        cfg.tags.retain(|t| t.id != id);
        confy::store_path(&self.tags_file, cfg)?;
        //Reload tags
        self.load_config()
    }

    pub fn get_tag(&self, id: TagId) -> Option<TagConfig> {
        self.tags.get(&id).cloned()
    }

    pub fn get_tags(&self) -> Vec<TagConfig> {
        self.tags.values().cloned().collect()
    }

    fn load_config(&mut self) -> Result<(), TaggingError> {
        debug!("Parsing tags from {:?}", self.tags_file);
        let cfg: TagsConfig = confy::load_path(&self.tags_file)?;
        //Clear previous tags
        self.tags.clear();
        for tag in cfg.tags {
            self.tags.insert(tag.id, tag);
        }
        info!("Loaded {} tags", self.tags.len());
        Ok(())
    }

    pub fn tag_document(&self, doc: &mut DocumentData) -> Result<(), TaggingError> {
        //TODO improve
        let mut ids: Vec<TagId> = vec![];
        for (_, tag_cfg) in self.tags.iter() {
            let tag = from_tag_config(tag_cfg).unwrap();
            if tag.matcher.match_document(&doc)? {
                doc.tags.push(tag.id);
                ids.push(tag.id);
            }
        }
        info!("Tagged document {} with tags {:?}", doc.id, ids);
        self.extract_meta(doc)?;
        self.infer_language(doc)?;
        Ok(())
    }

    fn extract_meta(&self, doc: &mut DocumentData) -> Result<(), TaggingError> {
        let parsed =
            commonregex::common_regex(doc.body.as_ref().ok_or(TaggingError::EmptyBody(doc.id))?);
        //Dates
        let dates = parsed.dates;
        debug!("Extracted dates from document {}: {:?}", doc.id, dates);
        if !dates.is_empty() {
            doc.extracted.doc_date =
                diligent_date_parser::parse_date(dates[0]).map(|d| d.with_timezone(&chrono::Utc));
            match &doc.extracted.doc_date {
                Some(d) => info!("Extracted date {} for document {}", &d, doc.id),
                None => info!("Could not extract date for document {}", doc.id),
            }
        }

        if self.extract_extended_metadata {
            //IBAN
            doc.extracted.iban = parsed.ibans.iter().map(|s| s.to_string()).collect();

            //Telephone numbers
            doc.extracted.phone = parsed
                .phones
                .iter()
                .map(|s| s.to_string())
                .filter(|t| !doc.extracted.iban.iter().any(|i| i.contains(t)))
                .collect();

            //E-Mail
            doc.extracted.email = parsed.emails.iter().map(|s| s.to_string()).collect();

            //Links
            doc.extracted.link = parsed
                .links
                .iter()
                .map(|s| s.to_string())
                .filter(|s| !doc.extracted.email.contains(s))
                .collect();
        }

        Ok(())
    }

    fn infer_language(&self, doc: &mut DocumentData) -> Result<(), TaggingError> {
        match detect(doc.body.as_ref().ok_or(TaggingError::EmptyBody(doc.id))?) {
            None => {
                info!("Could not infer language in document {}.", doc.id);
            }
            Some(info) => {
                doc.language = Some(info.lang().eng_name().into());
                info!(
                    "Document {} is written in {} with confidence of {}",
                    doc.id,
                    info.lang().eng_name(),
                    info.confidence()
                );
            }
        }

        Ok(())
    }
}

///////////////////////////////////// Tag ///////////////////////////////////////
pub type TagId = u64;

pub struct Tag {
    id: TagId,
    name: String,
    matcher: Box<dyn Matcher + Send + Sync>,
}

impl std::fmt::Debug for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("").field(&self.id).field(&self.name).finish()
    }
}

///////////////////////////////////// Tag config ///////////////////////////////////////

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
pub struct TagsConfig {
    curr_id: TagId,
    tags: Vec<TagConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TagConfig {
    pub id: TagId,
    pub name: String,
    pub color: Option<String>,
    pub matcher: MatcherConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum MatcherConfig {
    FullMatcher {
        match_str: String,
        case_insensitive: bool,
    },
    AnyMatcher {
        match_str: String,
        case_insensitive: bool,
    },
    RegexMatcher {
        match_str: String,
    },
    NoMatcher,
}

fn from_matcher_config(
    cfg: &MatcherConfig,
) -> Result<Box<dyn Matcher + Send + Sync>, TaggingError> {
    match cfg {
        MatcherConfig::FullMatcher {
            match_str,
            case_insensitive,
        } => Ok(Box::new(FullMatcher::new(match_str, *case_insensitive)?)),
        MatcherConfig::AnyMatcher {
            match_str,
            case_insensitive,
        } => Ok(Box::new(AnyMatcher::new(match_str, *case_insensitive)?)),
        MatcherConfig::RegexMatcher { match_str } => {
            Ok(Box::new(RegexMatcher::parse_string(match_str)?))
        }
        MatcherConfig::NoMatcher => Ok(Box::new(NoMatcher::new())),
    }
}

fn from_tag_config(cfg: &TagConfig) -> Result<Tag, TaggingError> {
    Ok(Tag {
        id: cfg.id,
        name: cfg.name.clone(),
        matcher: from_matcher_config(&cfg.matcher)?,
    })
}

#[cfg(test)]
mod tests {
    use super::{AnyMatcher, FullMatcher, Matcher, NoMatcher, RegexMatcher};
    use crate::index::document_repository::{DocumentData, ExtractedData};

    fn get_document_with_text(text: &str) -> DocumentData {
        DocumentData {
            id: 0,
            original_filename: String::new(),
            title: String::new(),
            body: Some(text.into()),
            tags: vec![],
            imported_date: chrono::Utc::now(),
            hash: String::new(),
            file_size: 0,
            language: None,
            extracted: ExtractedData::default(),
        }
    }

    #[test]
    fn nomatcher_test() {
        let matcher = NoMatcher::new();
        assert!(!matcher
            .match_document(&get_document_with_text("Test"))
            .unwrap());
        assert!(!matcher.match_document(&get_document_with_text("")).unwrap());
    }

    #[test]
    fn fullmatcher_test() {
        let sensitive_matcher = FullMatcher::new("test", false).unwrap();
        assert!(sensitive_matcher
            .match_document(&get_document_with_text("test"))
            .unwrap());
        assert!(!sensitive_matcher
            .match_document(&get_document_with_text("Test"))
            .unwrap());
        assert!(sensitive_matcher
            .match_document(&get_document_with_text("string test"))
            .unwrap());
        assert!(sensitive_matcher
            .match_document(&get_document_with_text("test string"))
            .unwrap());

        let insensitive_matcher = FullMatcher::new("test", true).unwrap();
        assert!(insensitive_matcher
            .match_document(&get_document_with_text("test"))
            .unwrap());
        assert!(insensitive_matcher
            .match_document(&get_document_with_text("Test"))
            .unwrap());
        assert!(insensitive_matcher
            .match_document(&get_document_with_text("string Test"))
            .unwrap());
        assert!(insensitive_matcher
            .match_document(&get_document_with_text("Test string"))
            .unwrap());

        let multiword_tester = FullMatcher::new("test string", false).unwrap();
        assert!(!multiword_tester
            .match_document(&get_document_with_text("test"))
            .unwrap());
        assert!(!multiword_tester
            .match_document(&get_document_with_text("Test"))
            .unwrap());
        assert!(!multiword_tester
            .match_document(&get_document_with_text("string Test"))
            .unwrap());
        assert!(multiword_tester
            .match_document(&get_document_with_text("test string"))
            .unwrap());
    }

    #[test]
    fn anymatcher_test() {
        let sensitive_matcher = AnyMatcher::new("test1,test2", false).unwrap();
        assert!(sensitive_matcher
            .match_document(&get_document_with_text("test1"))
            .unwrap());
        assert!(sensitive_matcher
            .match_document(&get_document_with_text("test2"))
            .unwrap());
        assert!(sensitive_matcher
            .match_document(&get_document_with_text("test1 test2"))
            .unwrap());
        assert!(!sensitive_matcher
            .match_document(&get_document_with_text("Test1"))
            .unwrap());
        assert!(!sensitive_matcher
            .match_document(&get_document_with_text("Test2"))
            .unwrap());
        assert!(!sensitive_matcher
            .match_document(&get_document_with_text("Test1 Test2"))
            .unwrap());

        let insensitive_matcher = AnyMatcher::new("test1,test2", true).unwrap();
        assert!(insensitive_matcher
            .match_document(&get_document_with_text("test1"))
            .unwrap());
        assert!(insensitive_matcher
            .match_document(&get_document_with_text("test2"))
            .unwrap());
        assert!(insensitive_matcher
            .match_document(&get_document_with_text("test1 test2"))
            .unwrap());
        assert!(insensitive_matcher
            .match_document(&get_document_with_text("Test1"))
            .unwrap());
        assert!(insensitive_matcher
            .match_document(&get_document_with_text("Test2"))
            .unwrap());
        assert!(insensitive_matcher
            .match_document(&get_document_with_text("Test1 Test2"))
            .unwrap());
    }

    #[test]
    fn regexmatcher_test() {
        let matcher = RegexMatcher::parse_string("\\d+").unwrap();
        assert!(matcher
            .match_document(&get_document_with_text("1234"))
            .unwrap());
        assert!(matcher
            .match_document(&get_document_with_text("1"))
            .unwrap());
        assert!(!matcher
            .match_document(&get_document_with_text("test"))
            .unwrap());
    }
}
