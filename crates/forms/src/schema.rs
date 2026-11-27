//! Typed, bounded synthetic form schema. No executable expressions or approval.
use finance_shared::money::{Amount, Money};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Type {
    Text,
    Decimal,
    Money,
    Date,
    Boolean,
    Enum,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Condition {
    field: String,
    equals: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Layout {
    row: u16,
    column: u8,
    width: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Field {
    key: String,
    #[serde(rename = "type")]
    ty: Type,
    required: bool,
    required_when_visible: bool,
    visible_if: Option<Condition>,
    options: Option<Vec<String>>,
    source_path: Option<String>,
    layout: Option<Layout>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    version: u64,
    fields: Vec<Field>,
}

pub struct Schema {
    version: u64,
    fields: BTreeMap<String, Field>,
    order: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidSchema,
    DuplicateField,
    Dependency,
    HiddenRequired,
    InvalidType,
    UnknownField,
    HiddenValue,
    MissingRequired,
    InvalidInput,
}

fn key_valid(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 64
        && key.as_bytes()[0].is_ascii_alphabetic()
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn source_path_valid(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 128
        && path.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}

fn typed(field: &Field, value: &Value) -> bool {
    match field.ty {
        Type::Text => value.as_str().is_some_and(|s| {
            !s.trim().is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control)
        }),
        Type::Decimal => value.as_str().is_some_and(|s| Amount::parse(s).is_ok()),
        Type::Money => serde_json::from_value::<Money>(value.clone()).is_ok(),
        Type::Date => value.as_str().is_some_and(|s| {
            s.len() == 10
                && !s.starts_with("0000")
                && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                    .is_ok_and(|d| d.to_string() == s)
        }),
        Type::Boolean => value.is_boolean(),
        Type::Enum => value.as_str().is_some_and(|s| {
            field
                .options
                .as_ref()
                .is_some_and(|o| o.iter().any(|v| v == s))
        }),
    }
}

pub fn compile(bytes: &[u8]) -> Result<Schema, Error> {
    if bytes.is_empty() || bytes.len() > 65_536 {
        return Err(Error::InvalidSchema);
    }
    let raw: Raw = serde_json::from_slice(bytes).map_err(|_| Error::InvalidSchema)?;
    if raw.version == 0
        || raw.version > 9_007_199_254_740_991
        || raw.fields.is_empty()
        || raw.fields.len() > 256
    {
        return Err(Error::InvalidSchema);
    }
    let mut fields = BTreeMap::new();
    let mut order = Vec::new();
    for field in raw.fields {
        if !key_valid(&field.key) {
            return Err(Error::InvalidSchema);
        }
        if field.required && field.visible_if.is_some() {
            return Err(Error::HiddenRequired);
        }
        if field
            .source_path
            .as_deref()
            .is_some_and(|path| !source_path_valid(path))
        {
            return Err(Error::InvalidSchema);
        }
        if field.layout.as_ref().is_some_and(|layout| {
            layout.column >= 12
                || layout.width == 0
                || layout.width > 12
                || u16::from(layout.column) + u16::from(layout.width) > 12
                || layout.row > 4095
        }) {
            return Err(Error::InvalidSchema);
        }
        if field.required_when_visible && (field.visible_if.is_none() || field.required) {
            return Err(Error::InvalidSchema);
        }
        match (&field.ty, &field.options) {
            (Type::Enum, Some(options))
                if !options.is_empty()
                    && options.len() <= 64
                    && options.iter().all(|v| {
                        !v.trim().is_empty() && v.len() <= 128 && !v.chars().any(char::is_control)
                    })
                    && options.iter().collect::<BTreeSet<_>>().len() == options.len() => {}
            (Type::Enum, _) | (_, Some(_)) => return Err(Error::InvalidSchema),
            _ => (),
        }
        // Conditions must depend on preceding fields, a bounded topological
        // order that rejects cycles and hidden controls from future nodes.
        if let Some(condition) = &field.visible_if {
            let previous = fields.get(&condition.field).ok_or(Error::Dependency)?;
            if !typed(previous, &condition.equals)
                || !matches!(previous.ty, Type::Boolean | Type::Enum)
            {
                return Err(Error::InvalidType);
            }
        }
        order.push(field.key.clone());
        if fields.insert(field.key.clone(), field).is_some() {
            return Err(Error::DuplicateField);
        }
    }
    Ok(Schema {
        version: raw.version,
        fields,
        order,
    })
}

impl Schema {
    pub fn version(&self) -> u64 {
        self.version
    }
    /// Returns ordered visible field keys. Missing optional control means
    /// dependent hidden; caller cannot submit hidden values or unknown fields.
    pub fn validate(&self, values: &BTreeMap<String, Value>) -> Result<Vec<String>, Error> {
        if values.len() > self.fields.len() || values.keys().any(|k| !self.fields.contains_key(k)) {
            return Err(Error::UnknownField);
        }
        let mut visible = BTreeSet::new();
        let mut ordered = Vec::new();
        for key in &self.order {
            let field = &self.fields[key];
            let show = field.visible_if.as_ref().is_none_or(|condition| {
                visible.contains(&condition.field)
                    && values.get(&condition.field) == Some(&condition.equals)
            });
            let value = values.get(key);
            if !show {
                if value.is_some() {
                    return Err(Error::HiddenValue);
                }
                continue;
            }
            if let Some(value) = value {
                if !typed(field, value) {
                    return Err(Error::InvalidType);
                }
            } else if field.required || field.required_when_visible {
                return Err(Error::MissingRequired);
            }
            visible.insert(key.clone());
            ordered.push(key.clone());
        }
        Ok(ordered)
    }
    pub fn validate_json(&self, bytes: &[u8]) -> Result<Vec<String>, Error> {
        if bytes.len() > 262_144 {
            return Err(Error::InvalidInput);
        }
        // Use a visitor to reject duplicate input keys rather than last-wins JSON.
        struct Values(BTreeMap<String, Value>);
        impl<'de> Deserialize<'de> for Values {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct Visitor;
                impl<'de> serde::de::Visitor<'de> for Visitor {
                    type Value = Values;
                    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        f.write_str("unique form field object")
                    }
                    fn visit_map<M: serde::de::MapAccess<'de>>(
                        self,
                        mut map: M,
                    ) -> Result<Values, M::Error> {
                        let mut values = BTreeMap::new();
                        while let Some((key, value)) = map.next_entry::<String, Value>()? {
                            if values.len() >= 256 || values.insert(key, value).is_some() {
                                return Err(serde::de::Error::custom(
                                    "duplicate or excessive form field",
                                ));
                            }
                        }
                        Ok(Values(values))
                    }
                }
                d.deserialize_map(Visitor)
            }
        }
        let values: Values = serde_json::from_slice(bytes).map_err(|_| Error::InvalidInput)?;
        self.validate(&values.0)
    }
}
