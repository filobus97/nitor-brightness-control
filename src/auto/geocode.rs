//! Place search answers from the Open-Meteo geocoding API (pure: URL building and parsing;
//! the HTTP request itself lives in `win::geo`).

use crate::i18n::Lang;
use crate::json::{Json, Parser, field, field_str};

/// Most results asked for and shown.
pub const MAX_RESULTS: usize = 5;
/// Longest query sent.
const MAX_QUERY_CHARS: usize = 80;

/// One search result.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    /// First-level administrative area (region, state).
    pub admin1: Option<String>,
    pub country: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub postcodes: Vec<String>,
}

impl Place {
    /// `Rome, Lazio, Italy`: the parts that exist, without immediate repeats.
    pub fn label(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for p in [
            Some(&self.name),
            self.admin1.as_ref(),
            self.country.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if parts.last() != Some(&p.as_str()) {
                parts.push(p);
            }
        }
        parts.join(", ")
    }
}

/// The request URL for `query` (names, and postcodes in many countries).
pub fn search_url(query: &str, lang: Lang) -> String {
    let query: String = query.trim().chars().take(MAX_QUERY_CHARS).collect();
    format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count={MAX_RESULTS}&language={}&format=json",
        percent_encode(&query),
        lang.as_setting()
    )
}

/// Percent-encode every UTF-8 byte except the RFC 3986 unreserved ones.
fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out += &format!("%{b:02X}");
        }
    }
    out
}

/// Whether the query looks like a bare postcode (digits and spaces only): the API
/// matches postcodes only for some countries, so an empty answer gets a hint.
pub fn is_numeric_query(query: &str) -> bool {
    let q = query.trim();
    !q.is_empty() && q.chars().all(|c| c.is_ascii_digit() || c == ' ')
}

/// The places in an API answer (at most [`MAX_RESULTS`]). No `results` key, which is how
/// the API says "nothing found", gives an empty list; `None` if the text is not JSON.
pub fn parse(json: &str) -> Option<Vec<Place>> {
    let Json::Obj(top) = Parser::new(json).value()? else {
        return None;
    };
    let mut places = Vec::new();
    if let Some(Json::Arr(items)) = field(&top, "results") {
        for item in items.iter().take(MAX_RESULTS) {
            let Json::Obj(o) = item else { continue };
            let (Some(Json::Num(latitude)), Some(Json::Num(longitude)), Some(name)) = (
                field(o, "latitude"),
                field(o, "longitude"),
                field_str(o, "name"),
            ) else {
                continue;
            };
            let postcodes = match field(o, "postcodes") {
                Some(Json::Arr(codes)) => codes
                    .iter()
                    .filter_map(|c| match c {
                        Json::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            places.push(Place {
                name,
                admin1: field_str(o, "admin1"),
                country: field_str(o, "country"),
                latitude: *latitude,
                longitude: *longitude,
                postcodes,
            });
        }
    }
    Some(places)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"results":[
{"id":3169070,"name":"Rome","latitude":41.89193,"longitude":12.51133,"elevation":20.0,
 "feature_code":"PPLC","country_code":"IT","admin1_id":3174976,"admin2_id":3169069,
 "timezone":"Europe/Rome","population":2872800,"postcodes":["00100","00118","00199"],
 "country_id":3175395,"country":"Italy","admin1":"Lazio","admin2":"Rome"},
{"id":4219762,"name":"Rome","latitude":34.25704,"longitude":-85.16467,"elevation":181.0,
 "feature_code":"PPLA2","country_code":"US","timezone":"America/New_York",
 "country_id":6252001,"country":"United States","admin1":"Georgia"},
{"id":1,"name":"Singapore","latitude":1.28967,"longitude":103.85007,"country":"Singapore","admin1":"Singapore"},
{"id":2,"name":"No coordinates"}
],"generationtime_ms":1.2}"#;

    #[test]
    fn parses_results() {
        let places = parse(SAMPLE).unwrap();
        assert_eq!(places.len(), 3);
        let rome = &places[0];
        assert_eq!(rome.name, "Rome");
        assert_eq!(rome.admin1.as_deref(), Some("Lazio"));
        assert_eq!(rome.country.as_deref(), Some("Italy"));
        assert_eq!((rome.latitude, rome.longitude), (41.89193, 12.51133));
        assert_eq!(rome.postcodes, ["00100", "00118", "00199"]);
        assert_eq!(rome.label(), "Rome, Lazio, Italy");
        assert_eq!(places[1].longitude, -85.16467);
        assert!(places[1].postcodes.is_empty());
        assert_eq!(places[2].label(), "Singapore");
    }

    #[test]
    fn empty_and_invalid_answers() {
        assert_eq!(parse(r#"{"generationtime_ms":0.4}"#), Some(Vec::new()));
        assert_eq!(parse(r#"{"results":[]}"#), Some(Vec::new()));
        assert_eq!(parse("not json"), None);
        assert_eq!(parse("[]"), None);
    }

    #[test]
    fn at_most_five() {
        let one = r#"{"name":"A","latitude":1,"longitude":2}"#;
        let json = format!(r#"{{"results":[{}]}}"#, [one; 8].join(","));
        assert_eq!(parse(&json).unwrap().len(), MAX_RESULTS);
    }

    #[test]
    fn url_is_encoded() {
        assert_eq!(
            search_url(" Reggio Emilia ", Lang::It),
            "https://geocoding-api.open-meteo.com/v1/search?name=Reggio%20Emilia&count=5&language=it&format=json"
        );
        assert!(search_url("Zürich & co", Lang::En).contains("name=Z%C3%BCrich%20%26%20co&"));
    }

    #[test]
    fn numeric_queries() {
        assert!(is_numeric_query("00100"));
        assert!(is_numeric_query(" 42 100 "));
        assert!(!is_numeric_query("SW1A"));
        assert!(!is_numeric_query("  "));
    }
}
