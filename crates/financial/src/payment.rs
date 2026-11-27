//! Net payment calculation using explicit, already-approved deduction treatment.
use crate::Error;
use finance_shared::money::Money;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Deduction {
    /// Explicit policy says this deduction is already reflected in eligible gross.
    Included,
    /// Explicit policy says this deduction does not apply.
    NotApplicable,
    /// A separately verified deduction; missing applicable amounts block.
    Separate(Option<Money>),
}

/// No tax inference, automatic rounding or implicit FX. The caller verifies
/// policy and source evidence before invoking the calculation.
pub fn net(
    gross: Option<Money>,
    discount: Deduction,
    retention: Deduction,
    advance: Deduction,
    withholding: Deduction,
) -> Result<Money, Error> {
    let gross = gross.ok_or(Error::Unknown)?;
    if gross.amount.decimal().is_sign_negative() {
        return Err(Error::Negative);
    }
    let mut value = gross;
    for deduction in [discount, retention, advance, withholding] {
        match deduction {
            Deduction::Included | Deduction::NotApplicable => (),
            Deduction::Separate(None) => return Err(Error::Unknown),
            Deduction::Separate(Some(amount)) => {
                if amount.currency != gross.currency {
                    return Err(Error::Currency);
                }
                if amount.amount.decimal().is_sign_negative() {
                    return Err(Error::Negative);
                }
                value = value.checked_sub(amount).map_err(|_| Error::Overflow)?;
                if value.amount.decimal().is_sign_negative() {
                    return Err(Error::ExceedsCap);
                }
            }
        }
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_shared::money::{Amount, Currency};

    fn m(n: &str, currency: &str) -> Money {
        Money {
            amount: Amount::parse(n).unwrap(),
            currency: Currency::parse(currency).unwrap(),
        }
    }

    #[test]
    fn no_double_discount_or_unknown_deduction() {
        use Deduction::*;
        let gross = Some(m("100", "VND"));
        assert_eq!(
            net(
                gross,
                Included,
                Separate(Some(m("10", "VND"))),
                Separate(Some(m("20", "VND"))),
                NotApplicable,
            ),
            Ok(m("70", "VND"))
        );
        assert_eq!(
            net(
                gross,
                Included,
                Separate(None),
                NotApplicable,
                NotApplicable
            ),
            Err(Error::Unknown)
        );
        assert_eq!(
            net(
                gross,
                Included,
                Separate(Some(m("101", "VND"))),
                NotApplicable,
                NotApplicable,
            ),
            Err(Error::ExceedsCap)
        );
        assert_eq!(
            net(
                gross,
                Included,
                Separate(Some(m("1", "USD"))),
                NotApplicable,
                NotApplicable,
            ),
            Err(Error::Currency)
        );
    }
}
