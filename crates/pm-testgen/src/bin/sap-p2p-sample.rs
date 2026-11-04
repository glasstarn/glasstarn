//! Writes made-up SAP purchase-to-pay tables for trials and demos.
//!
//! Usage: `sap-p2p-sample <dir> [orders] [seed] [flags]`;
//! `--cloud` writes the S/4HANA Public Cloud layout, `--ariba` adds Ariba reports
//! in front, `--o2c` writes order to cash instead of purchase to pay, with
//! `--salesforce` or `--sales-cloud` adding that front end before the orders.

fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    let cloud = args.iter().any(|a| a == "--cloud");
    let ariba = args.iter().any(|a| a == "--ariba");
    let o2c = args.iter().any(|a| a == "--o2c");
    let front = if args.iter().any(|a| a == "--salesforce") {
        Some(pm_testgen::sap_o2c::Front::Salesforce)
    } else if args.iter().any(|a| a == "--sales-cloud") {
        Some(pm_testgen::sap_o2c::Front::SalesCloud)
    } else {
        None
    };
    args.retain(|a| !a.starts_with("--"));
    let both = ariba && cloud;
    let Some(dir) = args.first() else {
        eprintln!(
            "usage: sap-p2p-sample <dir> [orders] [seed] [--cloud] [--ariba] [--o2c [--salesforce|--sales-cloud]]"
        );
        return std::process::ExitCode::from(2);
    };
    let mut settings = pm_testgen::sap_p2p::Settings::default();
    if let Some(orders) = args.get(1).and_then(|n| n.parse().ok()) {
        settings.orders = orders;
    }
    if let Some(seed) = args.get(2).and_then(|n| n.parse().ok()) {
        settings.seed = seed;
    }
    let dir = std::path::Path::new(dir);
    if o2c {
        let settings = pm_testgen::sap_o2c::Settings {
            orders: settings.orders,
            seed: settings.seed,
            ..Default::default()
        };
        let written = if let Some(front) = front {
            pm_testgen::sap_o2c::write_front(dir, &settings, front, cloud).map(|(truth, front)| {
                println!("{front:#?}");
                truth
            })
        } else if cloud {
            pm_testgen::sap_o2c::write_cloud(dir, &settings)
        } else {
            pm_testgen::sap_o2c::write(dir, &settings)
        };
        return match written {
            Ok(truth) => {
                println!("{truth:#?}");
                std::process::ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    let written = if ariba {
        let write = if both {
            pm_testgen::sap_p2p::write_ariba_cloud
        } else {
            pm_testgen::sap_p2p::write_ariba
        };
        write(dir, &settings).map(|(truth, ariba)| {
            println!("{ariba:#?}");
            truth
        })
    } else if cloud {
        pm_testgen::sap_p2p::write_cloud(dir, &settings)
    } else {
        pm_testgen::sap_p2p::write(dir, &settings)
    };
    match written {
        Ok(truth) => {
            println!("{truth:#?}");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
