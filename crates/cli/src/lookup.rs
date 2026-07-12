//! The only code in `epub-tailor` that opens a socket.
//!
//! It is deliberately quarantined here, behind the `online` feature, and it is
//! only ever reached from `epub-tailor metadata search|fetch`. **`fit`, `md` and
//! `check` never call into this module**, which is what lets converting a book
//! stay offline, deterministic and reproducible: the record is fetched by one
//! command and applied by another, with a file (or a pipe) in between.
//!
//! The parsing lives in `epub_tailor_core::metadata::openlibrary` - a pure
//! function from a response body to candidates, tested from recorded fixtures.
//! All that is here is the request.
//!
//! ## Being a good citizen of Open Library
//!
//! Open Library asks for 1 request/second from anonymous callers, and offers 3
//! to callers who identify themselves with a contact in the `User-Agent`. It
//! forbids bulk download and explicitly welcomes "human-facing discovery and
//! lookup services" - which is exactly one book at a time, when a person asks.
//! So: a real User-Agent, a hard gap between requests, short timeouts, and a
//! single retry when they say 429.

use std::time::{Duration, Instant};

use epub_tailor_core::metadata::MetadataDoc;
use epub_tailor_core::metadata::openlibrary::{self, Candidate};

/// Open Library's floor for anonymous callers is one request a second. We send a
/// User-Agent, which earns three, but a lookup makes at most two requests and a
/// person is waiting - so we simply stay under the strictest limit and never
/// have to think about it again.
const MIN_GAP: Duration = Duration::from_millis(1100);

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(20);

/// What Open Library asks for: a product, a version and a way to reach us.
/// `EPUB_TAILOR_USER_AGENT` overrides it, so a heavy user can add their own
/// contact address and earn the identified rate limit.
fn user_agent() -> String {
    std::env::var("EPUB_TAILOR_USER_AGENT").unwrap_or_else(|_| {
        format!(
            "epub-tailor/{} (+https://github.com/jordiboehme/epub-tailor)",
            env!("CARGO_PKG_VERSION")
        )
    })
}

/// A lookup session. Holds the clock that keeps us under Open Library's rate
/// limit across the two requests a full fetch makes.
pub struct Lookup {
    agent: ureq::Agent,
    last_request: Option<Instant>,
}

impl Lookup {
    pub fn new() -> Lookup {
        let config = ureq::Agent::config_builder()
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_global(Some(READ_TIMEOUT))
            .user_agent(user_agent())
            .build();
        Lookup {
            agent: config.new_agent(),
            last_request: None,
        }
    }

    /// Sleep out whatever is left of the minimum gap since the last request.
    fn throttle(&mut self) {
        if let Some(last) = self.last_request {
            let elapsed = last.elapsed();
            if elapsed < MIN_GAP {
                std::thread::sleep(MIN_GAP - elapsed);
            }
        }
        self.last_request = Some(Instant::now());
    }

    /// GET `url`, retrying once after a pause if Open Library says 429.
    fn get(&mut self, url: &str) -> Result<String, String> {
        for attempt in 0..2 {
            self.throttle();
            match self.agent.get(url).call() {
                Ok(mut response) => {
                    return response
                        .body_mut()
                        .read_to_string()
                        .map_err(|e| format!("could not read the response from {url}: {e}"));
                }
                // They asked us to slow down. Do exactly that, once.
                Err(ureq::Error::StatusCode(429)) if attempt == 0 => {
                    std::thread::sleep(Duration::from_secs(3));
                }
                Err(ureq::Error::StatusCode(429)) => {
                    return Err(
                        "Open Library is rate-limiting this machine (HTTP 429). Wait a minute and \
                         try again, or set EPUB_TAILOR_USER_AGENT to identify yourself for a \
                         higher limit."
                            .to_string(),
                    );
                }
                Err(ureq::Error::StatusCode(code)) => {
                    return Err(format!("Open Library answered {code} for {url}"));
                }
                Err(e) => {
                    return Err(format!("could not reach Open Library: {e}"));
                }
            }
        }
        unreachable!("the loop either returns or retries once")
    }

    /// Search Open Library. At least one of `title`/`author`/`isbn` must say
    /// something, or there is nothing to ask.
    pub fn search(
        &mut self,
        title: Option<&str>,
        author: Option<&str>,
        isbn: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Candidate>, String> {
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(isbn) = isbn.filter(|s| !s.trim().is_empty()) {
            query.push(("isbn", isbn.trim().to_string()));
        }
        if let Some(title) = title.filter(|s| !s.trim().is_empty()) {
            query.push(("title", title.trim().to_string()));
        }
        if let Some(author) = author.filter(|s| !s.trim().is_empty()) {
            query.push(("author", author.trim().to_string()));
        }
        if query.is_empty() {
            return Err(
                "nothing to search for: give a book to read the title and author from, or pass \
                 --title/--author/--isbn"
                    .to_string(),
            );
        }
        query.push(("limit", limit.clamp(1, 50).to_string()));
        // Ask only for the fields we map, which keeps their Solr honest and the
        // response small.
        query.push((
            "fields",
            "key,title,author_name,publisher,first_publish_year,subject,isbn,cover_i".to_string(),
        ));

        let url = format!("https://openlibrary.org/search.json?{}", encode(&query));
        let body = self.get(&url)?;
        openlibrary::parse_search(&body, title, author)
    }

    /// Fetch one complete record by its `openlibrary:OL…W` reference.
    ///
    /// The description and the full subject list live on the *work* record, not
    /// on the search doc, so this is the second request - and the reason `fetch`
    /// exists at all rather than `search` simply returning everything.
    pub fn fetch(&mut self, reference: &str) -> Result<(MetadataDoc, Option<String>), String> {
        let olid = openlibrary::olid_of(reference).ok_or_else(|| {
            format!("\"{reference}\" is not a reference I know; expected openlibrary:OL…")
        })?;

        // A work reference is what search hands out. Fetch it directly.
        let work_url = format!("https://openlibrary.org/works/{olid}.json");
        let work_body = self.get(&work_url)?;

        // The work record has the description and subjects but no title/authors
        // in a usable form, so search for the same key to fill the rest in.
        let mut doc = MetadataDoc::default();
        let mut cover = None;
        openlibrary::merge_work(&work_body, &mut doc, &mut cover)?;

        let search_url = format!(
            "https://openlibrary.org/search.json?q=key:/works/{olid}&fields=key,title,author_name,publisher,first_publish_year,subject,isbn,cover_i&limit=1"
        );
        if let Ok(body) = self.get(&search_url)
            && let Ok(candidates) = openlibrary::parse_search(&body, None, None)
            && let Some(best) = candidates.into_iter().next()
        {
            // The work's description and subjects win; everything else comes
            // from the search doc.
            let description = doc.description.take();
            let subjects = doc.subjects.take();
            doc = best.metadata;
            if description.is_some() {
                doc.description = description;
            }
            if subjects.is_some() {
                doc.subjects = subjects;
            }
            if cover.is_none() {
                cover = best.cover_url;
            }
        }

        Ok((doc, cover))
    }

    /// Download a cover image, returning its bytes and media type.
    ///
    /// Kept explicitly separate and opt-in: Open Library's *metadata* is CC0,
    /// but the cover *images* come from many sources and are not. The caller
    /// warns the user before this runs.
    pub fn fetch_cover(&mut self, url: &str) -> Result<(Vec<u8>, String), String> {
        self.throttle();
        let mut response = self
            .agent
            .get(url)
            .call()
            .map_err(|e| format!("could not download the cover from {url}: {e}"))?;

        let media_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or(v).trim().to_string())
            .filter(|v| v.starts_with("image/"))
            .unwrap_or_else(|| "image/jpeg".to_string());

        let mut data = Vec::new();
        response
            .body_mut()
            .with_config()
            .limit(16 * 1024 * 1024)
            .read_to_vec()
            .map(|bytes| data = bytes)
            .map_err(|e| format!("could not read the cover image: {e}"))?;

        if data.is_empty() {
            return Err("Open Library returned an empty cover image".to_string());
        }
        Ok((data, media_type))
    }
}

/// Percent-encode a query string. Small enough to do by hand, and it keeps a URL
/// crate out of the dependency tree.
fn encode(pairs: &[(&str, String)]) -> String {
    fn esc(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for byte in s.as_bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(*byte as char)
                }
                b' ' => out.push('+'),
                other => out.push_str(&format!("%{other:02X}")),
            }
        }
        out
    }
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", esc(k), esc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_is_percent_encoded() {
        let q = encode(&[
            ("title", "The Hobbit".to_string()),
            ("limit", "5".to_string()),
        ]);
        assert_eq!(q, "title=The+Hobbit&limit=5");
    }

    #[test]
    fn punctuation_is_escaped() {
        let q = encode(&[("author", "Ursula K. Le Guin & co".to_string())]);
        assert!(
            q.contains("%26"),
            "the ampersand must not split the query: {q}"
        );
    }

    #[test]
    fn the_user_agent_names_the_tool_and_where_to_find_it() {
        // Open Library asks callers to identify themselves; a fake browser UA is
        // exactly what they penalize.
        let ua = user_agent();
        assert!(ua.starts_with("epub-tailor/"), "got: {ua}");
        assert!(ua.contains("github.com"), "got: {ua}");
    }
}
