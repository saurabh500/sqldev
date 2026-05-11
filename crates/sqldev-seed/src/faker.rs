//! Tiny built-in faker: short curated lists + random helpers, no external
//! crate dependency. Supports the tags listed in [`FAKERS`].
//!
//! All RNG draws go through the caller's [`rand::Rng`] so determinism is
//! preserved.

use rand::Rng;
use rand::seq::SliceRandom;

/// Public registry of supported faker tags. Anything not in this list
/// returns `None` from [`generate`].
pub const FAKERS: &[&str] = &[
    "name",
    "first_name",
    "last_name",
    "email",
    "username",
    "word",
    "sentence",
    "paragraph",
    "city",
    "country",
    "state",
    "phone",
    "uuid",
    "guid",
    "int",
    "bool",
    "date",
    "time",
    "datetime",
];

const FIRST_NAMES: &[&str] = &[
    "Ada", "Ben", "Cara", "Dan", "Elena", "Finn", "Gita", "Hugo", "Iris", "Jules", "Kira", "Leo",
    "Mira", "Noah", "Owen", "Priya", "Quinn", "Rosa", "Sami", "Theo", "Una", "Vik", "Wren",
    "Xander", "Yara", "Zane",
];

const LAST_NAMES: &[&str] = &[
    "Alvarez", "Brooks", "Chen", "Davies", "Eriksen", "Fischer", "Garcia", "Hassan", "Iyer",
    "Johnson", "Kowalski", "Larsen", "Mueller", "Novak", "Okafor", "Patel", "Quigley", "Rossi",
    "Singh", "Tanaka", "Ueda", "Volkov", "Wright", "Xu", "Yamamoto", "Zhao",
];

const CITIES: &[&str] = &[
    "Seattle",
    "Austin",
    "Berlin",
    "Tokyo",
    "Madrid",
    "Lagos",
    "Mumbai",
    "Toronto",
    "Lima",
    "Cairo",
    "Helsinki",
    "Sydney",
    "Lisbon",
    "Boston",
    "Oslo",
    "Singapore",
    "Zurich",
    "Buenos Aires",
    "Vancouver",
    "Dublin",
];

const COUNTRIES: &[&str] = &[
    "USA",
    "Canada",
    "Germany",
    "Japan",
    "Spain",
    "Nigeria",
    "India",
    "Brazil",
    "Egypt",
    "Finland",
    "Australia",
    "Portugal",
    "Norway",
    "Singapore",
    "Switzerland",
    "Argentina",
    "Ireland",
];

const STATES: &[&str] = &[
    "WA", "OR", "CA", "TX", "NY", "MA", "FL", "IL", "CO", "OH", "GA", "PA", "NC", "VA", "AZ", "MN",
    "WI",
];

const WORDS: &[&str] = &[
    "lorem",
    "ipsum",
    "dolor",
    "sit",
    "amet",
    "consectetur",
    "adipiscing",
    "elit",
    "sed",
    "do",
    "eiusmod",
    "tempor",
    "incididunt",
    "ut",
    "labore",
    "magna",
    "aliqua",
    "enim",
    "minim",
    "veniam",
    "quis",
    "nostrud",
    "exercitation",
    "ullamco",
    "laboris",
    "nisi",
    "aliquip",
    "commodo",
];

/// Dispatch into the appropriate generator. Returns `None` for unknown
/// tags so callers can surface a `SeedError::UnknownFaker`.
pub fn generate<R: Rng + ?Sized>(
    tag: &str,
    rng: &mut R,
    len_hint: Option<usize>,
) -> Option<String> {
    let s = match tag {
        "first_name" => FIRST_NAMES.choose(rng)?.to_string(),
        "last_name" => LAST_NAMES.choose(rng)?.to_string(),
        "name" => format!("{} {}", FIRST_NAMES.choose(rng)?, LAST_NAMES.choose(rng)?),
        "email" => {
            // Append a 4-digit suffix so seeding moderately-sized tables
            // with a UNIQUE email column does not collide.
            let suffix = rng.gen_range(1000u32..10_000);
            let user = format!(
                "{}.{}{suffix}",
                FIRST_NAMES.choose(rng)?.to_ascii_lowercase(),
                LAST_NAMES.choose(rng)?.to_ascii_lowercase(),
            );
            format!("{user}@example.com")
        }
        "username" => format!(
            "{}{}",
            FIRST_NAMES.choose(rng)?.to_ascii_lowercase(),
            rng.gen_range(10u32..9_999),
        ),
        "word" => WORDS.choose(rng)?.to_string(),
        "sentence" => {
            let n = rng.gen_range(4..10);
            sentence(rng, n)
        }
        "paragraph" => {
            let n = rng.gen_range(2..5);
            (0..n)
                .map(|_| {
                    let len = rng.gen_range(4..10);
                    sentence(rng, len)
                })
                .collect::<Vec<_>>()
                .join(" ")
        }
        "city" => CITIES.choose(rng)?.to_string(),
        "country" => COUNTRIES.choose(rng)?.to_string(),
        "state" => STATES.choose(rng)?.to_string(),
        "phone" => format!(
            "+1-{:03}-{:03}-{:04}",
            rng.gen_range(200u32..999),
            rng.gen_range(200u32..999),
            rng.gen_range(0u32..9_999),
        ),
        "uuid" | "guid" => return Some(format!("'{}'", uuid_v4(rng))),
        "int" => return Some(rng.gen_range(0i64..1_000_000).to_string()),
        "bool" => {
            return Some(if rng.r#gen::<bool>() {
                "1".into()
            } else {
                "0".into()
            });
        }
        "date" => return Some(format!("'{}'", random_date(rng))),
        "time" => return Some(format!("'{}'", random_time(rng))),
        "datetime" => return Some(format!("'{} {}'", random_date(rng), random_time(rng))),
        _ => return None,
    };
    Some(quote_clipped(&s, len_hint))
}

/// Quote-and-escape a single faker word, honoring an optional max length.
pub fn quoted_word<R: Rng + ?Sized>(rng: &mut R, len_hint: Option<usize>) -> String {
    let w = WORDS.choose(rng).copied().unwrap_or("data");
    quote_clipped(w, len_hint)
}

fn sentence<R: Rng + ?Sized>(rng: &mut R, n: usize) -> String {
    let mut out = String::new();
    for i in 0..n {
        if i > 0 {
            out.push(' ');
        }
        let w = WORDS.choose(rng).copied().unwrap_or("lorem");
        if i == 0 {
            let mut chars = w.chars();
            if let Some(c) = chars.next() {
                out.extend(c.to_uppercase());
                out.push_str(chars.as_str());
            }
        } else {
            out.push_str(w);
        }
    }
    out.push('.');
    out
}

fn quote_clipped(s: &str, len_hint: Option<usize>) -> String {
    let trimmed = match len_hint {
        Some(n) if s.chars().count() > n => s.chars().take(n).collect::<String>(),
        _ => s.to_string(),
    };
    format!("N'{}'", trimmed.replace('\'', "''"))
}

/// RFC 4122 v4 UUID using the caller's RNG. Hex-formatted, no braces.
pub fn uuid_v4<R: Rng + ?Sized>(rng: &mut R) -> String {
    let mut bytes = [0u8; 16];
    rng.fill(&mut bytes);
    // Version 4
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    // Variant 1 (RFC 4122)
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

/// `YYYY-MM-DD` between 2000-01-01 and 2030-12-28 (avoids month-end edges
/// without a real calendar).
pub fn random_date<R: Rng + ?Sized>(rng: &mut R) -> String {
    let year = rng.gen_range(2000u32..=2030);
    let month = rng.gen_range(1u32..=12);
    let day = rng.gen_range(1u32..=28);
    format!("{year:04}-{month:02}-{day:02}")
}

/// `HH:MM:SS`.
pub fn random_time<R: Rng + ?Sized>(rng: &mut R) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        rng.gen_range(0u32..24),
        rng.gen_range(0u32..60),
        rng.gen_range(0u32..60),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha20Rng;

    #[test]
    fn every_listed_faker_returns_some() {
        let mut rng = ChaCha20Rng::seed_from_u64(7);
        for tag in FAKERS {
            assert!(generate(tag, &mut rng, Some(50)).is_some(), "tag={tag}");
        }
    }

    #[test]
    fn unknown_faker_returns_none() {
        let mut rng = ChaCha20Rng::seed_from_u64(7);
        assert!(generate("nope", &mut rng, None).is_none());
    }

    #[test]
    fn faker_outputs_are_quoted_strings_for_text_tags() {
        let mut rng = ChaCha20Rng::seed_from_u64(7);
        let v = generate("name", &mut rng, Some(20)).unwrap();
        assert!(v.starts_with("N'") && v.ends_with('\''));
    }

    #[test]
    fn uuid_v4_has_correct_shape() {
        let mut rng = ChaCha20Rng::seed_from_u64(7);
        let u = uuid_v4(&mut rng);
        assert_eq!(u.len(), 36);
        let parts: Vec<&str> = u.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            vec![8, 4, 4, 4, 12]
        );
        // Version 4 nibble.
        assert_eq!(&u[14..15], "4");
    }

    #[test]
    fn quote_clipped_truncates_to_len_hint() {
        let s = quote_clipped("abcdefghij", Some(3));
        assert_eq!(s, "N'abc'");
    }

    #[test]
    fn deterministic_across_seed() {
        let mut a = ChaCha20Rng::seed_from_u64(99);
        let mut b = ChaCha20Rng::seed_from_u64(99);
        assert_eq!(
            generate("email", &mut a, None),
            generate("email", &mut b, None)
        );
    }
}
