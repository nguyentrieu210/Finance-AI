//! Exact, policy-supplied tax, FX and signed adjustment arithmetic.
//! This module does not choose or approve any accounting policy or FX quote.
use crate::Error;
use finance_shared::money::{Amount, Currency, Money};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rounding {
    Exact,
    TowardZero,
    HalfAwayFromZero,
    HalfEven,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Precision {
    pub scale: u32,
    pub mode: Rounding,
}

/// Multiply two NUMERIC(28,8) values using integer arithmetic. Reject an
/// intermediate overflow instead of allowing Decimal to silently round it.
fn product(amount: Amount, rate: Amount, precision: Precision) -> Result<Amount, Error> {
    if precision.scale > 8 {
        return Err(Error::Unknown);
    }
    let a = amount.decimal();
    let b = rate.decimal();
    let numerator = a
        .mantissa()
        .checked_mul(b.mantissa())
        .ok_or(Error::Overflow)?;
    let input_scale = a.scale() + b.scale();
    let negative = numerator < 0;
    let mut units = numerator.checked_abs().ok_or(Error::Overflow)?;
    if input_scale > precision.scale {
        let divisor = 10_i128.pow(input_scale - precision.scale);
        let remainder = units % divisor;
        units /= divisor;
        let twice = remainder.checked_mul(2).ok_or(Error::Overflow)?;
        let increment = match precision.mode {
            Rounding::Exact if remainder != 0 => return Err(Error::Unknown),
            Rounding::Exact | Rounding::TowardZero => false,
            Rounding::HalfAwayFromZero => twice >= divisor,
            Rounding::HalfEven => twice > divisor || (twice == divisor && units % 2 != 0),
        };
        if increment {
            units = units.checked_add(1).ok_or(Error::Overflow)?;
        }
    } else {
        units = units
            .checked_mul(10_i128.pow(precision.scale - input_scale))
            .ok_or(Error::Overflow)?;
    }
    let factor = 10_i128.pow(precision.scale);
    let sign = if negative && units != 0 { "-" } else { "" };
    let value = if precision.scale == 0 {
        format!("{sign}{units}")
    } else {
        format!(
            "{sign}{}.{:0width$}",
            units / factor,
            units % factor,
            width = precision.scale as usize
        )
    };
    Amount::parse(&value).map_err(|_| Error::Overflow)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaxBreakdown {
    pub net: Money,
    pub tax: Money,
    pub gross: Money,
}

/// The rate is a fraction, e.g. 0.1 for 10%. None is unknown, not zero tax.
pub fn tax(net: Money, rate: Option<Amount>, precision: Precision) -> Result<TaxBreakdown, Error> {
    let rate = rate.ok_or(Error::Unknown)?;
    if net.amount.decimal().is_sign_negative() || rate.decimal().is_sign_negative() {
        return Err(Error::Negative);
    }
    let tax = Money {
        amount: product(net.amount, rate, precision)?,
        currency: net.currency,
    };
    let gross = net.checked_add(tax).map_err(|_| Error::Overflow)?;
    Ok(TaxBreakdown { net, tax, gross })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedQuote {
    pub from: Currency,
    pub to: Currency,
    /// Units of target currency per one unit of source currency.
    pub rate: Amount,
}

/// The caller must verify quote provenance, effective date and approval first.
pub fn convert(
    source: Money,
    quote: Option<VerifiedQuote>,
    precision: Precision,
) -> Result<Money, Error> {
    let quote = quote.ok_or(Error::Unknown)?;
    if quote.from != source.currency {
        return Err(Error::Currency);
    }
    if quote.rate.decimal() <= 0.into() {
        return Err(Error::Negative);
    }
    Ok(Money {
        amount: product(source.amount, quote.rate, precision)?,
        currency: quote.to,
    })
}

/// Apply an explicitly approved signed adjustment without hiding an overdraft.
pub fn adjust(base: Money, adjustment: Option<Money>) -> Result<Money, Error> {
    let adjustment = adjustment.ok_or(Error::Unknown)?;
    if base.currency != adjustment.currency {
        return Err(Error::Currency);
    }
    if base.amount.decimal().is_sign_negative() {
        return Err(Error::Negative);
    }
    let result = base.checked_add(adjustment).map_err(|_| Error::Overflow)?;
    if result.amount.decimal().is_sign_negative() {
        return Err(Error::Negative);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn amount(value: &str) -> Amount {
        Amount::parse(value).unwrap()
    }

    fn money(value: &str, currency: &str) -> Money {
        Money {
            amount: amount(value),
            currency: Currency::parse(currency).unwrap(),
        }
    }

    fn precision(mode: Rounding) -> Precision {
        Precision { scale: 0, mode }
    }

    #[test]
    fn e11_tax_rounding_is_explicit_and_missing_rates_block() {
        let net = money("105", "VND");
        let rate = Some(amount("0.1"));
        assert_eq!(
            tax(net, rate, precision(Rounding::HalfAwayFromZero))
                .unwrap()
                .gross,
            money("116", "VND")
        );
        assert_eq!(
            tax(net, rate, precision(Rounding::HalfEven)).unwrap().tax,
            money("10", "VND")
        );
        assert_eq!(
            tax(net, rate, precision(Rounding::Exact)),
            Err(Error::Unknown)
        );
        assert_eq!(
            tax(net, None, precision(Rounding::HalfEven)),
            Err(Error::Unknown)
        );
    }

    #[test]
    fn e11_fx_requires_a_quote_in_the_correct_direction() {
        let source = money("-1.25", "USD");
        let quote = VerifiedQuote {
            from: Currency::parse("USD").unwrap(),
            to: Currency::parse("VND").unwrap(),
            rate: amount("10"),
        };
        assert_eq!(
            convert(source, Some(quote), precision(Rounding::HalfAwayFromZero)),
            Ok(money("-13", "VND"))
        );
        assert_eq!(
            convert(money("1", "EUR"), Some(quote), precision(Rounding::Exact)),
            Err(Error::Currency)
        );
        assert_eq!(
            convert(source, None, precision(Rounding::Exact)),
            Err(Error::Unknown)
        );
    }

    #[test]
    fn e11_signed_adjustments_and_overflow_are_checked() {
        assert_eq!(
            adjust(money("100", "VND"), Some(money("-20", "VND"))),
            Ok(money("80", "VND"))
        );
        assert_eq!(
            adjust(money("100", "VND"), Some(money("-101", "VND"))),
            Err(Error::Negative)
        );
        assert_eq!(
            adjust(money("100", "VND"), Some(money("1", "USD"))),
            Err(Error::Currency)
        );
        assert_eq!(
            product(
                amount("99999999999999999999"),
                amount("99999999999999999999"),
                precision(Rounding::Exact)
            ),
            Err(Error::Overflow)
        );
    }
}
