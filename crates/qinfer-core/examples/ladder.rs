//! Run the setup ladder against this machine and print every event.
//!
//! `cargo run --release -p qinfer-core --example ladder`
//!
//! This is the same sequence the dashboard consumes on `Enter`, rendered to
//! stdout instead of a panel — which is the point of ADR-0009: one event stream,
//! two surfaces, no second implementation to drift.

use qinfer_core::catalog::Catalog;
use qinfer_core::setup::{self, SetupEvent, SystemMeasurements};
use qinfer_core::sweetspot::DEFAULT_USABLE_TPS;
use qinfer_core::microops::Outcome;

const CATALOG: &str = include_str!("../../../catalog/qinfer-catalog-2026.10.27.json");

fn main() {
    let catalog = match Catalog::from_json(CATALOG) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("catalogue did not parse: {e}");
            std::process::exit(2);
        }
    };

    println!("ladder · {:.0} tok/s floor\n", DEFAULT_USABLE_TPS);
    let m = SystemMeasurements;

    setup::run(&m, &catalog, DEFAULT_USABLE_TPS, |e| match e {
        SetupEvent::Begin(s) => println!("     .. {}  (started)", s.label()),
        SetupEvent::Stage(p) => println!("     ok {:<9} {}", p.stage.label(), p.detail),
        SetupEvent::Measured(r) => {
            for op in &r.ops {
                match &op.outcome {
                    Outcome::Measured { value, unit, spread_pct, .. } => {
                        println!("        {:<26} {value:>9.2} {unit:<8} spread {spread_pct:.1}%", op.id.label())
                    }
                    Outcome::Refused { reason, .. } => {
                        println!("        {:<26} -- {reason}", op.id.label())
                    }
                }
            }
        }
        SetupEvent::Budget(s) => println!("        budget   {}", s.describe()),
        SetupEvent::Fingerprint(_) => {}
        SetupEvent::Chosen(s) => {
            println!("\n     choice   {}", s.usable_reason(DEFAULT_USABLE_TPS));
            for n in &s.notes {
                println!("     note     {n}");
            }
        }
        SetupEvent::Failed { stage, reason } => {
            println!("     FAILED {}: {reason}", stage.label())
        }
        SetupEvent::Done => println!("\n     done"),
    });
}
