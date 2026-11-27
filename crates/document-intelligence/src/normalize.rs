//! Locale-explicit candidate normalization. Never infer currency or timezone.
use chrono::NaiveDate;
use finance_shared::money::Amount;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    Vietnamese,
    English,
    Unspecified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome<T> {
    Unknown,
    Invalid,
    Ambiguous,
    Candidate(T),
}

/// Original OCR string is retained verbatim, including separators and spaces.
/// No Debug implementation: raw values must not be included in logs.
pub struct Normalized<'a, T> {
    pub raw: &'a str,
    pub outcome: Outcome<T>,
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn decimal(raw: &str, group: char, fraction: char) -> Option<Amount> {
    let (sign, unsigned) = if let Some(rest) = raw.strip_prefix('-') {
        ("-", rest)
    } else {
        ("", raw)
    };
    let (whole, fractional) = match unsigned.split_once(fraction) {
        Some((a, b)) if digits(b) && b.len() <= 8 => (a, Some(b)),
        Some(_) => return None,
        None => (unsigned, None),
    };
    let mut groups = whole.split(group);
    let first = groups.next()?;
    if !digits(first) {
        return None;
    }
    let mut canonical = format!("{sign}{first}");
    for (index, part) in groups.enumerate() {
        if (index == 0 && first.len() > 3) || part.len() != 3 || !digits(part) {
            return None;
        }
        canonical.push_str(part);
    }
    if let Some(f) = fractional {
        canonical.push('.');
        canonical.push_str(f);
    }
    Amount::parse(&canonical).ok()
}

fn choose<T: Eq + Copy>(a: Option<T>, b: Option<T>, format: Format) -> Outcome<T> {
    match format {
        Format::Vietnamese => a.map_or(Outcome::Invalid, Outcome::Candidate),
        Format::English => b.map_or(Outcome::Invalid, Outcome::Candidate),
        Format::Unspecified => match (a, b) {
            (Some(x), Some(y)) if x == y => Outcome::Candidate(x),
            (Some(_), Some(_)) => Outcome::Ambiguous,
            (Some(x), None) | (None, Some(x)) => Outcome::Candidate(x),
            (None, None) => Outcome::Invalid,
        },
    }
}

/// No currency conversion, rounding, exponent, embedded spaces, currency
/// symbol removal, O/0 correction or sign inference. Unknown locale must
/// not turn 1.234 or 1,234 into a silently chosen amount.
pub fn amount(raw: &str, format: Format) -> Normalized<'_, Amount> {
    let value = raw.trim();
    let outcome = if raw.len() > 128 {
        Outcome::Invalid
    } else if value.is_empty() {
        Outcome::Unknown
    } else {
        choose(decimal(value, '.', ','), decimal(value, ',', '.'), format)
    };
    Normalized { raw, outcome }
}

fn calendar_date(value: &str, day_first: bool) -> Option<NaiveDate> {
    let parts: Vec<_> = value.split('/').collect();
    let [a, b, year] = parts.as_slice() else {
        return None;
    };
    if !digits(a) || !digits(b) || !digits(year) || a.len() > 2 || b.len() > 2 || year.len() != 4 {
        return None;
    }
    let a = a.parse().ok()?;
    let b = b.parse().ok()?;
    let year = year.parse().ok()?;
    if year == 0 {
        return None;
    }
    let (day, month) = if day_first { (a, b) } else { (b, a) };
    NaiveDate::from_ymd_opt(year, month, day)
}

pub fn date(raw: &str, format: Format) -> Normalized<'_, NaiveDate> {
    let value = raw.trim();
    let outcome = if raw.len() > 64 {
        Outcome::Invalid
    } else if value.is_empty() {
        Outcome::Unknown
    } else if value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
        && !value.starts_with("0000")
    {
        NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .ok()
            .map_or(Outcome::Invalid, Outcome::Candidate)
    } else {
        choose(
            calendar_date(value, true),
            calendar_date(value, false),
            format,
        )
    };
    Normalized { raw, outcome }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_keep_ambiguity_and_exact_precision() {
        let raw = " 1.234 ";
        let result = amount(raw, Format::Unspecified);
        assert_eq!(result.raw, raw);
        assert_eq!(result.outcome, Outcome::Ambiguous);
        assert_eq!(
            amount("1,234", Format::Unspecified).outcome,
            Outcome::Ambiguous
        );
        assert_eq!(
            amount("1.234,50", Format::Vietnamese).outcome,
            Outcome::Candidate(Amount::parse("1234.50").unwrap())
        );
        assert_eq!(
            amount("1,234.50", Format::English).outcome,
            Outcome::Candidate(Amount::parse("1234.50").unwrap())
        );
        for bad in ["12.34.567", "1e6", "O123", "1 234", "1,2,3", "100đ"] {
            assert_eq!(amount(bad, Format::Unspecified).outcome, Outcome::Invalid);
        }
        assert_eq!(amount("", Format::Unspecified).outcome, Outcome::Unknown);
        assert_eq!(
            amount("0", Format::Unspecified).outcome,
            Outcome::Candidate(Amount::parse("0").unwrap())
        );
    }

    #[test]
    fn dates_require_valid_calendar_and_preserve_ambiguity() {
        assert_eq!(
            date("01/02/2027", Format::Unspecified).outcome,
            Outcome::Ambiguous
        );
        let february = NaiveDate::from_ymd_opt(2027, 2, 1).unwrap();
        assert_eq!(
            date("01/02/2027", Format::Vietnamese).outcome,
            Outcome::Candidate(february)
        );
        assert_eq!(
            date("2027-02-01", Format::Unspecified).outcome,
            Outcome::Candidate(february)
        );
        for bad in [
            "29/02/2027",
            "31/04/2027",
            "01/02/27",
            "0000-01-01",
            "2027-01-01T00:00Z",
        ] {
            assert_eq!(date(bad, Format::Unspecified).outcome, Outcome::Invalid);
        }
        assert_eq!(date("", Format::Unspecified).outcome, Outcome::Unknown);
    }
}
