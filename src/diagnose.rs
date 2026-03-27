//! CLI `diagnose` subcommand — prints the same checks as Telegram `/self`.

use crate::config::Config;
use crate::diagnose_core::{DiagReport, DiagSeverity};

/// Print structured report to stdout (human-readable).
pub async fn run(config: &Config) {
    println!("clos-monitor diagnose");
    println!("=====================");
    println!("Checking the full Telegram webhook chain...");

    let report = crate::diagnose_core::run_full_diagnostic(config).await;
    print_report(&report);
    print_summary(&report);
}

fn print_report(report: &DiagReport) {
    for sec in &report.sections {
        println!("\n--- {} ---", sec.title);
        for line in &sec.lines {
            let tag = match line.severity {
                DiagSeverity::Ok => "[OK]  ",
                DiagSeverity::Warn => "[WARN]",
                DiagSeverity::Fail => "[FAIL]",
            };
            println!("{} {}", tag, line.message);
            if let Some(d) = &line.detail {
                for l in d.lines() {
                    println!("       {}", l);
                }
            }
        }
    }
}

fn print_summary(report: &DiagReport) {
    let total = report.passed + report.failed + report.warned;
    println!("\n=============================================");
    if report.failed == 0 && report.warned == 0 {
        println!(
            "Summary: {}/{} checks passed — all good!",
            report.passed, total
        );
    } else {
        println!(
            "Summary: {} passed, {} failed, {} warnings (total {})",
            report.passed, report.failed, report.warned, total
        );
    }
    if report.failed > 0 {
        println!("\nFAILED checks indicate a broken link in the webhook chain.");
        println!("See DEPLOY_CHECKLIST.md or run 'make monitor-webhook-set' to fix registration.");
    }
}
