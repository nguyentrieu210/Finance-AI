use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Storage compatibility with NUMERIC(28,8); no implicit rounding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Amount(Decimal);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoneyError {
    InvalidDecimal,
    OutOfRange,
    InvalidCurrency,
    CurrencyMismatch,
    Overflow,
}

impl Amount {
    pub fn from_decimal(value: Decimal) -> Result<Self, MoneyError> {
        // Normalize only trailing zeros; this does not round. Reject precision beyond the column.
        let value = value.normalize();
        if value.scale() > 8
            || value.abs() >= Decimal::from_i128_with_scale(100_000_000_000_000_000_000, 0)
        {
            return Err(MoneyError::OutOfRange);
        }
        Ok(Self(value))
    }
    pub fn parse(value: &str) -> Result<Self, MoneyError> {
        let unsigned = value.strip_prefix('-').unwrap_or(value);
        let (integer, fraction) = match unsigned.split_once('.') {
            Some((i, f)) if !f.is_empty() && f.len() <= 8 => (i, f),
            Some(_) => return Err(MoneyError::InvalidDecimal),
            None => (unsigned, ""),
        };
        if integer.is_empty()
            || !integer.bytes().all(|b| b.is_ascii_digit())
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(MoneyError::InvalidDecimal);
        }
        if integer.trim_start_matches('0').len() > 20 {
            return Err(MoneyError::OutOfRange);
        }
        let parsed = Decimal::from_str_exact(value).map_err(|_| MoneyError::InvalidDecimal)?;
        Self::from_decimal(parsed)
    }
    pub fn decimal(self) -> Decimal {
        self.0
    }
    pub fn checked_add(self, rhs: Self) -> Result<Self, MoneyError> {
        Self::from_decimal(self.0.checked_add(rhs.0).ok_or(MoneyError::Overflow)?)
    }
    pub fn checked_sub(self, rhs: Self) -> Result<Self, MoneyError> {
        Self::from_decimal(self.0.checked_sub(rhs.0).ok_or(MoneyError::Overflow)?)
    }
}
impl Serialize for Amount {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.to_string())
    }
}
impl<'de> Deserialize<'de> for Amount {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = String::deserialize(d)?;
        Self::parse(&value).map_err(|_| serde::de::Error::custom("invalid decimal amount"))
    }
}

/// Syntactic three-letter code. Active ISO currency registry is a policy/reference-data concern.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Currency([u8; 3]);
impl Currency {
    pub fn parse(value: &str) -> Result<Self, MoneyError> {
        let bytes: [u8; 3] = value
            .as_bytes()
            .try_into()
            .map_err(|_| MoneyError::InvalidCurrency)?;
        if !bytes.iter().all(u8::is_ascii_uppercase) {
            return Err(MoneyError::InvalidCurrency);
        }
        Ok(Self(bytes))
    }
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).expect("validated ASCII currency")
    }
}
impl Serialize for Currency {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}
impl<'de> Deserialize<'de> for Currency {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(d)?)
            .map_err(|_| serde::de::Error::custom("invalid currency"))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Money {
    pub amount: Amount,
    pub currency: Currency,
}
impl Money {
    pub fn checked_add(self, rhs: Self) -> Result<Self, MoneyError> {
        if self.currency != rhs.currency {
            return Err(MoneyError::CurrencyMismatch);
        }
        Ok(Self {
            amount: self.amount.checked_add(rhs.amount)?,
            currency: self.currency,
        })
    }
    pub fn checked_sub(self, rhs: Self) -> Result<Self, MoneyError> {
        if self.currency != rhs.currency {
            return Err(MoneyError::CurrencyMismatch);
        }
        Ok(Self {
            amount: self.amount.checked_sub(rhs.amount)?,
            currency: self.currency,
        })
    }
}
