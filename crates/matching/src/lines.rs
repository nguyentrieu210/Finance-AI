//! Bounded exact reconciliation of explicitly paired lines. No fuzzy allocation,
//! implicit unit conversion, source authority, or reservation effect.
use finance_shared::{ids::{CompanyScope, ResourceId}, money::{Amount, Money}};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    UnknownCoverage,
    Scope,
    InvalidLine,
    DuplicateLine,
    InvalidAllocation,
    Incompatible,
    ExceedsQuantity,
    ExceedsAmount,
    Overflow,
}

pub struct Line<'a> {
    pub id: ResourceId,
    pub scope: CompanyScope,
    pub item: &'a str,
    pub unit: &'a str,
    pub quantity: Amount,
    pub value: Money,
}

pub struct Allocation {
    pub source: ResourceId,
    pub target: ResourceId,
    pub quantity: Amount,
    pub value: Money,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Residual {
    pub id: ResourceId,
    pub quantity: Amount,
    pub value: Money,
}

pub struct Reconciliation {
    pub source_remaining: Vec<Residual>,
    pub target_remaining: Vec<Residual>,
}

impl Reconciliation {
    /// A complete target is not an authoritative financial approval.
    pub fn target_complete(&self) -> bool {
        self.target_remaining.iter().all(|l| l.quantity.decimal()==0.into() && l.value.amount.decimal()==0.into())
    }
}

fn valid(line: &Line<'_>) -> bool {
    !line.item.trim().is_empty() && line.item.len()<=128
        && !line.unit.trim().is_empty() && line.unit.len()<=32
        && line.quantity.decimal()>0.into() && line.value.amount.decimal()>=0.into()
}

fn residual(line: &Line<'_>) -> Residual {
    Residual { id:line.id,quantity:line.quantity,value:line.value }
}

fn subtract(r: &mut Residual, a: &Allocation) -> Result<(), Error> {
    let quantity=r.quantity.checked_sub(a.quantity).map_err(|_|Error::Overflow)?;
    let value=r.value.checked_sub(a.value).map_err(|_|Error::Overflow)?;
    if quantity.decimal()<0.into() {return Err(Error::ExceedsQuantity);}
    if value.amount.decimal()<0.into(){return Err(Error::ExceedsAmount);}
    r.quantity=quantity;
    r.value=value;
    Ok(())
}

/// Source lines already carry remaining eligible balances, not gross values
/// that include previously consumed quantities. A caller must prove complete
/// source coverage and current scope before invoking this pure function.
pub fn reconcile(
    scope:CompanyScope,
    sources:&[Line<'_>],
    targets:&[Line<'_>],
    allocations:&[Allocation],
    complete:bool,
)->Result<Reconciliation,Error>{
    if !complete {return Err(Error::UnknownCoverage);}
    if sources.is_empty() || targets.is_empty() || sources.len()>256 || targets.len()>256 || allocations.len()>1024 {
        return Err(Error::InvalidLine);
    }
    let mut source=BTreeMap::new();
    let mut target=BTreeMap::new();
    let mut ids=BTreeSet::new();
    for (lines,index) in [(sources,&mut source),(targets,&mut target)]{
        for line in lines {
            if line.scope!=scope{return Err(Error::Scope);}
            if !valid(line){return Err(Error::InvalidLine);}
            if !ids.insert(line.id){return Err(Error::DuplicateLine);}
            index.insert(line.id,(line,residual(line)));
        }
    }
    let mut edges=BTreeSet::new();
    for a in allocations {
        if a.quantity.decimal()<=0.into() || a.value.amount.decimal()<0.into()
            || !edges.insert((a.source,a.target)){return Err(Error::InvalidAllocation);}
        let (s,sr)=source.get_mut(&a.source).ok_or(Error::InvalidAllocation)?;
        let (t,tr)=target.get_mut(&a.target).ok_or(Error::InvalidAllocation)?;
        if s.item!=t.item || s.unit!=t.unit || s.value.currency!=t.value.currency || s.value.currency!=a.value.currency {
            return Err(Error::Incompatible);
        }
        subtract(sr,a)?;
        subtract(tr,a)?;
    }
    Ok(Reconciliation{
        source_remaining:source.into_values().map(|(_,r)|r).collect(),
        target_remaining:target.into_values().map(|(_,r)|r).collect(),
    })
}

#[cfg(test)]
mod tests{
    use super::*;
    use finance_shared::{ids::{CompanyId,TenantId}, money::Currency};
    use std::str::FromStr;

    fn id(n:u128)->ResourceId{
        ResourceId::from_str(&format!("00000000-0000-0000-0000-{n:012x}")).unwrap()
    }
    fn scope()->CompanyScope{
        CompanyScope{
            tenant:TenantId::from_str("00000000-0000-0000-0000-000000000001").unwrap(),
            company:CompanyId::from_str("00000000-0000-0000-0000-000000000002").unwrap(),
        }
    }
    fn qty(n:&str)->Amount{Amount::parse(n).unwrap()}
    fn money(n:&str)->Money{Money{amount:qty(n),currency:Currency::parse("VND").unwrap()}}
    fn line(n:u128,q:&str,v:&str,unit:&'static str)->Line<'static>{
        Line{id:id(n),scope:scope(),item:"SYN-ITEM",unit,quantity:qty(q),value:money(v)}
    }
    fn edge(s:u128,t:u128,q:&str,v:&str)->Allocation{
        Allocation{source:id(s),target:id(t),quantity:qty(q),value:money(v)}
    }
    #[test]
    fn many_to_many_split_and_unmatched(){
        let sources=[line(10,"4","40","KG"),line(11,"6","60","KG")];
        let targets=[line(20,"5","50","KG"),line(21,"3","30","KG")];
        let a=[edge(10,20,"4","40"),edge(11,20,"1","10"),edge(11,21,"3","30")];
        let result=reconcile(scope(),&sources,&targets,&a,true).unwrap();
        assert!(result.target_complete());
        assert_eq!(result.source_remaining[1].quantity,qty("2"));
        assert_eq!(result.source_remaining[1].value,money("20"));
        assert!(!reconcile(scope(),&sources,&targets,&a[..1],true).unwrap().target_complete());
        assert_eq!(reconcile(scope(),&sources,&targets,&a,false).err(),Some(Error::UnknownCoverage));
    }
    #[test]
    fn rejects_duplicate_overallocation_and_unit_mismatch(){
        let sources=[line(10,"4","40","KG")];
        let targets=[line(20,"3","30","KG"),line(21,"3","30","KG")];
        assert_eq!(reconcile(scope(),&sources,&targets,&[edge(10,20,"3","30"),edge(10,21,"2","20")],true).err(),Some(Error::ExceedsQuantity));
        assert_eq!(reconcile(scope(),&sources,&targets,&[edge(10,20,"1","10"),edge(10,20,"1","10")],true).err(),Some(Error::InvalidAllocation));
        assert_eq!(reconcile(scope(),&sources,&targets,&[edge(10,20,"1","31")],true).err(),Some(Error::ExceedsAmount));
        assert_eq!(reconcile(scope(),&sources,&[line(20,"1","10","BOX")],&[edge(10,20,"1","10")],true).err(),Some(Error::Incompatible));
    }
}
