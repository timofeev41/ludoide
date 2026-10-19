//! Search syntax for the Databases panel.
//!
//! - Words separated by `|` or spaces are alternatives: `ad|timofeev` matches
//!   `adhub_marts` *and* `timofeev_devel`.
//! - A plain word matches database names; if it matches no database at all,
//!   it searches table/view names instead (see [`Search::resolve`]).
//! - `db.table` narrows: `ad.events` = objects containing "events" in databases
//!   containing "ad"; `ad.` = databases only; `.events` = objects in any database.
//! - Matching is case-insensitive substring.

#[derive(Clone, Debug, PartialEq)]
struct Term {
    db: Option<String>,
    obj: Option<String>,
    /// Written without a dot: matches either a database or an object.
    either: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Search {
    terms: Vec<Term>,
}

/// Which objects of a database to show.
#[derive(Debug, PartialEq)]
pub enum ObjectFilter {
    All,
    /// Object names containing any of these.
    Names(Vec<String>),
    None,
}

impl ObjectFilter {
    pub fn matches(&self, name: &str) -> bool {
        match self {
            ObjectFilter::All => true,
            ObjectFilter::Names(n) => {
                let l = name.to_lowercase();
                n.iter().any(|t| l.contains(t))
            }
            ObjectFilter::None => false,
        }
    }
}

fn opt(s: &str) -> Option<String> {
    let s = s.trim().to_lowercase();
    (!s.is_empty()).then_some(s)
}

impl Search {
    pub fn parse(q: &str) -> Self {
        let terms = q
            .split(|c: char| c == '|' || c.is_whitespace())
            .filter(|t| !t.trim().is_empty() && t.trim() != ".")
            .map(|t| match t.split_once('.') {
                Some((d, o)) => Term { db: opt(d), obj: opt(o), either: false },
                None => Term { db: opt(t), obj: opt(t), either: true },
            })
            .collect();
        Self { terms }
    }

    /// Decide what each plain word means for this connection: a database search
    /// if some database contains it, otherwise a table search.
    pub fn resolve(&self, databases: &[String]) -> Search {
        let terms = self
            .terms
            .iter()
            .map(|t| {
                if !t.either {
                    return t.clone();
                }
                let w = t.db.clone().unwrap_or_default();
                if databases.iter().any(|d| d.to_lowercase().contains(&w)) {
                    Term { db: Some(w), obj: None, either: false }
                } else {
                    Term { db: None, obj: Some(w), either: false }
                }
            })
            .collect();
        Search { terms }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// True if any term looks for objects (so table names are worth indexing).
    pub fn wants_objects(&self) -> bool {
        self.terms.iter().any(|t| t.obj.is_some())
    }

    /// The database name itself matches a term.
    pub fn db_matches(&self, db: &str) -> bool {
        let l = db.to_lowercase();
        self.terms
            .iter()
            .any(|t| t.db.as_ref().is_some_and(|d| l.contains(d.as_str())) && (t.either || t.obj.is_none()))
    }

    /// Objects to show inside `db`.
    pub fn objects(&self, db: &str) -> ObjectFilter {
        if self.terms.is_empty() {
            return ObjectFilter::All;
        }
        let l = db.to_lowercase();
        let db_ok = |d: &Option<String>| d.as_ref().is_none_or(|d| l.contains(d.as_str()));
        let mut names = Vec::new();
        for t in &self.terms {
            if t.either {
                if db_ok(&t.db) {
                    return ObjectFilter::All;
                }
                names.extend(t.obj.clone());
            } else if db_ok(&t.db) {
                match &t.obj {
                    None => return ObjectFilter::All,
                    Some(o) => names.push(o.clone()),
                }
            }
        }
        if names.is_empty() { ObjectFilter::None } else { ObjectFilter::Names(names) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternatives() {
        let s = Search::parse("ad|timofeev");
        assert!(s.db_matches("adhub_marts") && s.db_matches("timofeev_devel") && !s.db_matches("system"));
        assert_eq!(s.objects("adhub_marts"), ObjectFilter::All);
        // In other databases, plain words still find tables by name.
        assert_eq!(s.objects("system"), ObjectFilter::Names(vec!["ad".into(), "timofeev".into()]));
        assert!(Search::parse("a  b").terms.len() == 2);
        assert!(Search::parse("").is_empty());
    }

    #[test]
    fn plain_words_prefer_databases() {
        let dbs = vec!["adhub_marts".to_owned(), "timofeev_devel".to_owned(), "system".to_owned()];
        let s = Search::parse("ad|timofeev").resolve(&dbs);
        assert!(s.db_matches("adhub_marts") && s.db_matches("timofeev_devel"));
        // "system" has tables containing "ad", but databases matched, so it stays hidden.
        assert_eq!(s.objects("system"), ObjectFilter::None);
        assert!(!s.wants_objects());
        // No database contains "events": search tables everywhere.
        let s = Search::parse("events").resolve(&dbs);
        assert!(!s.db_matches("system"));
        assert_eq!(s.objects("system"), ObjectFilter::Names(vec!["events".into()]));
        assert!(s.wants_objects());
    }

    #[test]
    fn dotted() {
        let s = Search::parse("ad.events");
        assert!(!s.db_matches("adhub"));
        assert_eq!(s.objects("adhub"), ObjectFilter::Names(vec!["events".into()]));
        assert_eq!(s.objects("system"), ObjectFilter::None);
        assert!(Search::parse("ad.").db_matches("adhub"));
        assert_eq!(Search::parse("ad.").objects("adhub"), ObjectFilter::All);
        assert_eq!(Search::parse(".Events").objects("any"), ObjectFilter::Names(vec!["events".into()]));
        assert!(ObjectFilter::Names(vec!["events".into()]).matches("raw_EVENTS"));
    }
}
