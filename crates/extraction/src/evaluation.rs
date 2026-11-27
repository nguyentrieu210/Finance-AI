//! Deterministic E08 golden-corpus evaluation. This scores extraction
//! candidates only; a candidate is never a verified field or financial approval.
use crate::{Field, State, fields};
use finance_document_intelligence::Kind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Expected {
    Candidate,
    NoCandidate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Case {
    pub kind: Kind,
    pub field: Field,
    pub expected: Expected,
    pub observed: State,
    pub latency_ms: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Provider-neutral integer billing unit supplied by the caller.
    pub cost_microunits: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    EmptyCorpus,
    UnsupportedPair,
    InvalidTelemetry,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PairMetrics {
    pub kind: Kind,
    pub field: Field,
    pub cases: u32,
    pub positive_cases: u32,
    pub negative_cases: u32,
    pub false_acceptances: u32,
    pub misses: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Report {
    pub cases: u32,
    pub positive_cases: u32,
    pub negative_cases: u32,
    pub false_acceptances: u32,
    pub misses: u32,
    pub p95_latency_ms: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_microunits: u64,
    pub coverage_complete: bool,
    pub pairs: Vec<PairMetrics>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_false_acceptance_bps: u32,
    pub max_miss_bps: u32,
    pub max_p95_latency_ms: u32,
    pub max_cost_microunits_per_case: u64,
}

fn all_kinds() -> [Kind; 8] {
    [
        Kind::Contract,
        Kind::ContractAppendix,
        Kind::Invoice,
        Kind::PaymentRequest,
        Kind::AdvanceRequest,
        Kind::AdvanceSettlement,
        Kind::AcceptanceRecord,
        Kind::PaymentVoucher,
    ]
}

pub fn required_pairs() -> Vec<(Kind, Field)> {
    let mut pairs = Vec::new();
    for kind in all_kinds() {
        pairs.extend(fields(kind).iter().copied().map(|field| (kind, field)));
    }
    pairs
}

fn supported(kind: Kind, field: Field) -> bool {
    fields(kind).contains(&field)
}

fn rate_bps(numerator: u32, denominator: u32) -> u32 {
    if denominator == 0 {
        0
    } else {
        ((u64::from(numerator) * 10_000) / u64::from(denominator)) as u32
    }
}

impl Report {
    pub fn false_acceptance_bps(&self) -> u32 {
        rate_bps(self.false_acceptances, self.negative_cases)
    }

    pub fn miss_bps(&self) -> u32 {
        rate_bps(self.misses, self.positive_cases)
    }

    pub fn passes(&self, limits: Limits) -> bool {
        if !self.coverage_complete || self.cases == 0 {
            return false;
        }
        let average_cost = self.cost_microunits / u64::from(self.cases);
        self.false_acceptance_bps() <= limits.max_false_acceptance_bps
            && self.miss_bps() <= limits.max_miss_bps
            && self.p95_latency_ms <= limits.max_p95_latency_ms
            && average_cost <= limits.max_cost_microunits_per_case
    }
}

pub fn evaluate(cases: &[Case]) -> Result<Report, Error> {
    if cases.is_empty() {
        return Err(Error::EmptyCorpus);
    }
    let required = required_pairs();
    let mut pairs = required
        .iter()
        .map(|&(kind, field)| PairMetrics {
            kind,
            field,
            cases: 0,
            positive_cases: 0,
            negative_cases: 0,
            false_acceptances: 0,
            misses: 0,
        })
        .collect::<Vec<_>>();
    let mut latencies = Vec::with_capacity(cases.len());
    let mut positive_cases = 0_u32;
    let mut negative_cases = 0_u32;
    let mut false_acceptances = 0_u32;
    let mut misses = 0_u32;
    let mut input_tokens = 0_u64;
    let mut output_tokens = 0_u64;
    let mut cost_microunits = 0_u64;

    for case in cases {
        if !supported(case.kind, case.field) {
            return Err(Error::UnsupportedPair);
        }
        if case.latency_ms > 300_000
            || case.input_tokens > 1_000_000
            || case.output_tokens > 1_000_000
            || case.cost_microunits > 1_000_000_000_000
        {
            return Err(Error::InvalidTelemetry);
        }
        let pair = pairs
            .iter_mut()
            .find(|metric| metric.kind == case.kind && metric.field == case.field)
            .expect("supported pair is in required pairs");
        pair.cases += 1;
        match case.expected {
            Expected::Candidate => {
                positive_cases += 1;
                pair.positive_cases += 1;
                if case.observed != State::Candidate {
                    misses += 1;
                    pair.misses += 1;
                }
            }
            Expected::NoCandidate => {
                negative_cases += 1;
                pair.negative_cases += 1;
                if case.observed == State::Candidate {
                    false_acceptances += 1;
                    pair.false_acceptances += 1;
                }
            }
        }
        latencies.push(case.latency_ms);
        input_tokens += u64::from(case.input_tokens);
        output_tokens += u64::from(case.output_tokens);
        cost_microunits = cost_microunits
            .checked_add(case.cost_microunits)
            .ok_or(Error::InvalidTelemetry)?;
    }
    latencies.sort_unstable();
    let rank = (latencies.len() * 95).div_ceil(100).max(1) - 1;
    let coverage_complete = pairs.iter().all(|metric| metric.cases > 0);
    Ok(Report {
        cases: u32::try_from(cases.len()).map_err(|_| Error::InvalidTelemetry)?,
        positive_cases,
        negative_cases,
        false_acceptances,
        misses,
        p95_latency_ms: latencies[rank],
        input_tokens,
        output_tokens,
        cost_microunits,
        coverage_complete,
        pairs,
    })
}
