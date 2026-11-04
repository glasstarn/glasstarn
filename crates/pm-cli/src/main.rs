//! `glasstarn` — the command-line interface.
//!
//! This binary contains no analysis logic. It parses arguments, calls
//! `pm-query`, and formats the result. The browser service and the MCP server
//! will call exactly the same functions, so all three can never disagree about
//! a number.

mod doctor;
mod setup;
mod update;
mod workspace;

use pm_core::OcelFilter;
use pm_core::automation::ResourceKind;
use pm_core::filter::Filter;
use pm_ingest::Rules;
use pm_ingest::finding::{Kind, Policy};
use pm_ingest::validate::Mapping;
use pm_project::{Project, Source};
use pm_query::{
    Analysis, AnalysisCache, CacheKey, LabelledProcessTree, Outcome, analyze_filtered, brief,
};
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}

/// The file a person named, when a quick command reads a converted copy of it.
static ORIGINAL_FILE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn run(args: &[String]) -> Result<ExitCode, String> {
    if args.is_empty() {
        return serve_command(&[]);
    }
    // "Is glasstarn ready?": agents check the install with this.
    if args[0] == "--version" || args[0] == "-V" || args[0] == "version" {
        println!("glasstarn {}", env!("CARGO_PKG_VERSION"));
        return Ok(ExitCode::SUCCESS);
    }
    if args[0] == "help" || args[0] == "--help" || args[0] == "-h" {
        if args.get(1).is_some_and(|a| a == "--all" || a == "all") {
            print_usage();
        } else {
            print_short_usage();
        }
        return Ok(ExitCode::SUCCESS);
    }

    // The quick one-file commands read Excel and other spreadsheet formats
    // the same way a project import does.
    let mut args = args.to_vec();
    let _converted = if matches!(
        args[0].as_str(),
        "analyze"
            | "conformance"
            | "discover"
            | "inspect"
            | "quality"
            | "case"
            | "cases"
            | "resources"
            | "automation"
            | "compare-cohorts"
            | "compare-periods"
            | "values"
    ) && args
        .get(1)
        .is_some_and(|file| std::path::Path::new(file).is_file())
    {
        let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
        if let Some(read) = workspace::readable_file(std::path::Path::new(&args[1]), dir.path())? {
            let _ = ORIGINAL_FILE.set(args[1].clone());
            args[1] = read.display().to_string();
        }
        Some(dir)
    } else {
        None
    };
    match args[0].as_str() {
        "analyze" => analyze_command(&args[1..]),
        "conformance" => conformance_command(&args[1..]),
        "doctor" => doctor::command(&args[1..]),
        "discover" => discover_command(&args[1..]),
        "import-bpmn" => import_bpmn_command(&args[1..]),
        "inspect" => inspect_command(&args[1..]),
        "ocel" => ocel_command(&args[1..]),
        "quality" => quality_command(&args[1..]),
        "case" => case_command(&args[1..]),
        "cases" => cases_command(&args[1..]),
        "resources" => resources_command(&args[1..]),
        "automation" => automation_command(&args[1..]),
        "compare-cohorts" => compare_cohorts_command(&args[1..]),
        "compare-periods" => compare_periods_command(&args[1..]),
        "values" => values_command(&args[1..]),
        "mcp" => mcp_command(&args[1..]),
        "mcp-policy" => mcp_policy_command(&args[1..]),
        "mcp-preview" => mcp_preview_command(&args[1..]),
        "serve" => serve_command(&args[1..]),
        "workspace" => workspace::command(&args[1..]),
        "setup" => setup::command(&args[1..]),
        "open" => open_command(&args[1..]),
        "update" => update::command(&args[1..]),
        "uninstall" => setup::uninstall(&args[1..]),
        // `glasstarn demo <name>` is the short form people type.
        "demo" => workspace::command(&[vec!["demo".to_owned()], args[1..].to_vec()].concat()),
        "extract" => workspace::extract::command(&args[1..]),
        "formats" => formats_command(&args[1..]),
        other => Err(format!("unknown command '{other}'. Try `glasstarn help`.")),
    }
}

/// The few commands people type. Everything else is for agents and scripts,
/// listed by `glasstarn help --all`.
fn print_short_usage() {
    println!(
        "glasstarn — local process mining

  glasstarn                  Open glasstarn in your browser
  glasstarn setup            Connect glasstarn to Codex and Claude Code
  glasstarn update           Install the latest version
  glasstarn uninstall        Remove glasstarn from this computer (keeps projects)
  glasstarn demo <name>      Make a demo project: purchase-to-pay, order-to-cash,
                             connected or sales-pipeline
  glasstarn formats          File types glasstarn can read
  glasstarn doctor           Check that everything is set up and up to date
  glasstarn help --all       Every command, for agents and scripts

Most people never need more than this. In your agent (Claude Code, Codex),
type /glasstarn or just ask, for example \"why are our invoices paid late?\"."
    );
}

fn print_usage() {
    println!(
        "glasstarn — local process mining

USAGE
  glasstarn inspect <file.csv|file.xlsx|file.parquet|file.xes|file.xes.gz> [--json]
  glasstarn quality <file.csv|file.xlsx|file.parquet|file.xes|file.xes.gz> [--json]
  glasstarn analyze <file.csv|file.xlsx|file.parquet|file.xes|file.xes.gz> [options]
  glasstarn ocel <file.jsonocel|file.xmlocel|file.sqlite|file.sqlite3> [options]
  glasstarn discover <file.csv|file.xlsx|file.parquet|file.xes|file.xes.gz> [options]
  glasstarn conformance <file.csv|file.xlsx|file.parquet|file.xes|file.xes.gz> --model <model.json> [options]
  glasstarn import-bpmn <model.bpmn> [--json]
  glasstarn case <file> <case-id> [--json]
  glasstarn cases <file> --activity <name> [--json]
  glasstarn cases <file> --transition <from> <to> [--json]
  glasstarn cases <file> --variant 'A > B > C' [--json]
  glasstarn resources <file> [--rules <rules.json>] [--json]
  glasstarn automation <file> [--rules <rules.json>] [--json]
  glasstarn compare-cohorts <file> --attribute <name> --left <value> --right <value> [--json]
  glasstarn values <file> --field <name> [--by variant|last-step|<attribute>] [--limit <n>] [--json]
  glasstarn compare-periods <file> --baseline-start <micros> --baseline-end <micros> --comparison-start <micros> --comparison-end <micros> [--json]
  glasstarn mcp [--project-dir <directory>] [--live-port <port>] [--connection <session.json>] [--project-id <id>] [--exposure-policy <policy.json>]
  glasstarn mcp-policy <policy.json>
  glasstarn mcp-preview <project.json> [--exposure-policy <policy.json>] [--json]
  glasstarn serve [--port <port>] [--workspace-dir <directory>] [--project <id>] [--no-open] [--agent-access <policy.json>]
  glasstarn workspace <create|list|inspect|inventory|prepare|prepare-ocel|import|analyze|query|record|window|name-type|show|export> [options] [--full]
  glasstarn extract salesforce <project-id|folder> --spec <spec.json> [--workspace-dir <directory>]
  glasstarn extract hubspot <project-id|folder> --spec <spec.json>   (token from HUBSPOT_TOKEN)
  glasstarn extract servicenow <project-id|folder> --spec <spec.json>   (SERVICENOW_USER, SERVICENOW_PASSWORD)
  glasstarn extract jira <project-id|folder> --spec <spec.json>   (JIRA_EMAIL, JIRA_API_TOKEN)
  glasstarn extract discover <project-id|folder> ...   (what a Salesforce org, Jira site or ServiceNow instance holds)
  glasstarn extract status <project-id|folder> [--workspace-dir <directory>]
  glasstarn setup
  glasstarn doctor [--json]   (checks the version, connected agents, running pages and project folders)
  glasstarn update
  glasstarn uninstall [--delete-projects]
  glasstarn open [--workspace-dir <directory>] [--project <id>]   (browser view in the background; prints its link)
  glasstarn demo <purchase-to-pay|order-to-cash|connected|sales-pipeline>
  glasstarn formats [--json]
  glasstarn help [--all]

LOCAL BROWSER SERVICE
  Running `glasstarn` with no command starts the browser service on
  127.0.0.1, chooses a free port, and opens the authenticated local UI.
  `serve --no-open` prints the URL without opening a browser, for headless use.

UPDATES
  At most once a day glasstarn asks registry.npmjs.org for the latest version
  number, to mention an update; nothing about you or your projects is sent.
  Turn it off with GLASSTARN_NO_UPDATE_CHECK=1 or \"update_check\": false in
  ~/.glasstarn/settings.json.

LOCAL MCP SERVER
  `mcp` runs a read-only newline-delimited JSON-RPC server over stdin/stdout.
  Agents can discover saved projects and inspect bounded log metadata without
  receiving raw events or sample values. The project directory defaults to the
  current directory.

  Before connecting an AI, create and review a policy with `mcp-policy`, then
  inspect exactly how identifiers will be transformed with `mcp-preview`.
  To query the exact analysis open in the browser, start `serve` with
  `--agent-access <policy>` and configure MCP with the project session’s `--live-port` and `--project-id` plus
  the same policy. Live access is otherwise disabled.

HEADLESS PROJECT WORKFLOW
  `workspace` lets an approved agent complete a ready-made-log investigation or
  prepare approved CSV extracts without a browser. Commands emit compact JSON,
  held to about 4 KB by cutting long lists (each cut list is followed by a
  <name>_omitted count); add --full for everything. Query results are also
  saved whole in the project.

  workspace create --name <name> [--purpose audit|improvement]
  workspace demo <purchase-to-pay|order-to-cash|connected|sales-pipeline>   (invented data; see docs/demos.md)
  workspace demo bpic2019 <BPI_Challenge_2019.xes>   (five-minute demo; see docs/demo-bpic2019.md)
  workspace decide <project> rule|performer|value-field ... --status suggested|confirmed|rejected --by <who>
  workspace pack list | checklist <pack>
  workspace pack handout <project> --pack <pack> [--from DATE --to DATE --scope VALUE --variant ID --note TEXT]
  workspace pack combine <project> --pack <first> --with <second> --link <table>:<target>
  workspace pack check|answer|build <project> --pack <pack> ...   (see docs/process-packs.md)
  workspace list
  workspace show <project-id|folder>
  workspace export <project-id|folder> [--type <item type>]   (process map as BPMN, items and variants as CSV, current filters, into the project's exports folder; --type for linked records)
  workspace inspect <source>
  workspace inventory <project-id|folder> <extract.csv> [more extracts]
  workspace prepare <project-id|folder> --recipe <recipe.json>
  workspace import <project-id|folder> <source> [mapping and decision options]
  workspace analyze <project-id|folder>
  workspace query <project-id|folder> --request <query.json>
  workspace record <project-id|folder> --request <investigation.json>
  workspace window <project-id|folder> [--start <rfc3339> --end <rfc3339>] [--end-step <step>]... [--clear]
  workspace name-type <project-id|folder> --type <item type> (--name <plural business name> | --clear)
  workspace move <project-id|folder> --to <folder>   (moves a project, with everything saved in it)

  Project commands default to the user-owned application directory; pass
  --workspace-dir <directory> to use another approved project location for any
  workspace command. Inventory copies extracts into the named project; prepare
  applies only the bounded, reviewed recipe contract and then runs analysis.
  A case import accepts --case, --activity, --timestamp,
  --event-order, --end-timestamp, --resource, --lifecycle, --cost,
  --rules and repeatable --answer <kind>=<choice>. Every other column is kept
  as a case attribute; repeatable --leave-out <column> drops one, and
  repeatable --case-attribute keeps only the ones listed.
  OCEL import needs no case-column mapping. An exit status of 3 carries a JSON
  result with mapping or quality decisions that require confirmation.

CASE AND CASES
  `case` opens one case in full: its ordered events, timings, resources,
  costs, attributes, and the source row each event came from.

  `cases` lists the cases behind a finding, so any figure leads to its
  evidence. Use --activity, --transition <from> <to>, --handover
  <from> <to>, or --variant 'A > B > C'. Add --limit <n> (default 20).

  Both accept the same preparation options as `analyze` — --project,
  --answer, --rename, --rules and the rest. Evidence produced under
  different settings than the analysis would not be evidence for it.
  They also accept the same view filters, including --where,
  --with-activity, --top-variants and --complete-only.

INSPECT
  Shows every column with sample values and what it could be mapped to.
  Useful before analysing a file with many columns. Use --json for columns,
  ranked role candidates, unfillable fields, and the proposed mapping.

QUALITY
  Produces a first-class data-quality report: every validation finding with
  severity, affected rows and cases, examples, open decisions, and supplied
  answers. Use --json for the shared machine-readable representation.

OBJECT-CENTRIC ANALYSIS
  `ocel` analyzes an OCEL 2.0 JSON, XML, or SQLite log without flattening events into cases.
  It reports quality, filtered scope, lifecycle flow per object type, qualified
  participation, shared-event interactions, explicit object relationships,
  and bounded evidence. Use --json for the complete machine-readable result.

  --object-type <name>       Keep objects of this type. May be repeated.
  --e2o-qualifier <name>     Keep event-to-object roles. May be repeated.
  --o2o-qualifier <name>     Keep object-to-object roles. May be repeated.
  --bottlenecks              Rank lifecycle elapsed and convergence spread burden.
  --project-as <object type> Explicitly preview this type as a case notion.
  --case-limit <n>           Projected cases to show (default 5, maximum 100).
  --event-limit <n>          Events to show per projected case (default 100, maximum 1000).
  --discover-model           Discover a native typed object-centric Petri net.
  --noise-threshold <percent>  Local infrequent-edge threshold (default 20).
  --variable-arc-threshold <percent>  Fixed-arc confidence threshold (default 100).
  --conformance-model <file> Check this log against an explicit saved model JSON.
  --conformance-algorithm <name>  token (fast default) or alignment (optimal A*).
  --detail-limit <n>         Worst objects and deviations to show (default 100, maximum 10000).

AUTOMATION DISCOVERY
  `resources` describes every recorded performer and shows—but never applies—a
  conservative human/system suggestion. Confirm classifications in a rules file.

  `automation` then reports straight-through coverage, manual-work hotspots,
  separate processing and pre-step delay, routing contexts, dominant-variant
  divergences, and example cases. It orders investigation targets; it does not
  decide that a task should be automated.

COMPARISON
  `values` totals a number field such as an invoice amount: total, average,
  median and 90th percentile, overall or by route, last step or another
  attribute. Cases without a number are counted, never treated as zero. With
  confirmed finishing steps, it also reports the amount still open.

  `compare-cohorts` compares two values of a mapped case attribute inside the
  active view. It reports denominators and ranked activity differences as
  statistical associations, never as causal effects.

  `compare-periods` compares two non-overlapping inclusive case-start windows.
  Boundaries are Unix microseconds, matching JSON and MCP without timezone guesses.

ANALYZE OPTIONS
  --case <column>        Column holding the case identifier
  --activity <column>    Column holding the activity name
  --timestamp <column>   Column holding the timestamp
                         (all three are proposed automatically if omitted)

  --answer <kind>=<option>   Answer a decision the validation raised.
                             May be repeated. Run without it first to see
                             which decisions apply.
  --json                 Emit JSON instead of a readable report
  --brief                Emit a markdown brief to paste into an AI assistant

FORMAL DISCOVERY
  `discover` produces a sound process tree and accepting Petri net with the
  Inductive Miner. It accepts the same mapping, preparation, project, and view
  options as `analyze`.

  --algorithm <name>         imf (sound default), alpha, or heuristics.
  --noise-threshold <percent>  Local infrequent-edge threshold (default 20).
                               Decimal tenths are accepted, e.g. 12.5.
  --json                       Emit the complete tree and Petri net as JSON.

CONFORMANCE
  `conformance` replays this log against an explicit formal model previously
  saved by `discover --json`. It never derives the baseline from the same log.

  --model <model.json>  Required saved formal model.
  --algorithm <name>    token (fast default) or alignment (optimal A*).
  --limit <n>           Worst deviating cases to return (default 20).
  --json                Emit bounded, source-linked deviation evidence as JSON.

BPMN IMPORT
  `import-bpmn` converts documented BPMN tasks, start/end events, exclusive
  gateways, and parallel gateways into the same accepting Petri-net JSON used
  by conformance. Unsupported constructs are rejected explicitly.

VIEW FILTERS
  Filters choose which cases to look at. They never change the data, and the
  share of cases shown is always reported.

  --with-activity <name>     Only cases that contain this activity.
  --without-activity <name>  Only cases that do not.
  --starts-with <name>       Only cases whose first recorded event is this activity.
  --ends-with <name>         Only cases whose last recorded event is this activity.
  --directly-follows <a> <b> Only cases where a is immediately followed by b.
  --eventually-follows <a> <b>
                             Only cases where a occurs before a later b.
  --where <attr>=<value>     Only cases whose attribute matches, e.g.
                             --where region=EMEA. For a column that changes
                             between steps (a status), cases that ever had it.
  --latest <attr>=<value>    Only cases whose value on their last step is this,
                             e.g. --latest status=Blocked (still blocked).
  --repeats <name|any>       Only cases that record this step, or any step,
                             more than once.
  --handled-by <name>        Only cases where this person recorded a step.
  --where '<attr>>=<number>' Only cases whose attribute is a number at least
  --where '<attr><=<number>' (or at most) the bound, e.g. --where 'amount>=50000'.
                             Cases without a number never match.
  --top-variants <n>         Only the n most common paths. The quickest way to
                             make a crowded map readable.
  --min-case-events <n>      Only cases with at least n events.
  --slower-than <duration>   Only cases taking longer, e.g. 2d, 6h, 30m.
  --faster-than <duration>   Only cases taking less.
  --complete-only            Drop cases flagged as missing data.

  Trace order means the normalized recorded event sequence. Equal-time events
  use the confirmed ordering policy or source-row tie-breaker; matching them
  does not establish causality.

PROJECTS
  --save-project <file.json> Record the mapping, answers, and rules, plus a
                             fingerprint of the source file. Completed results
                             are cached in a disposable sidecar directory.
  --project <file.json>      Reopen a saved project. The file argument becomes
                             optional; flags still override what was saved and
                             receive their own correctly keyed cache entry.

PREPARATION RULES
  These never modify your file. They are applied at analysis time and are
  recorded with the project, so results stay reproducible.

  --rename <old>=<new>       Rename an activity. Repeat to merge several
                             spellings onto one name.
  --exclude-activity <name>  Drop an activity. Its neighbours join directly.
  --exclude-case <id>        Drop one case, e.g. test data.
  --min-events <n>           Drop cases with fewer than n events.
  --drop-incomplete          Drop cases flagged as missing data.
  --rules <file.json>        Load a rule set from a file. Individual flags
                             above refine it. An AI assistant can propose one
                             from the brief; you review it before it applies.

EXAMPLES
  glasstarn analyze orders.csv
  glasstarn analyze orders.csv --answer duplicate_rows=deduplicate
  glasstarn analyze orders.csv --json > analysis.json
  glasstarn analyze orders.csv --brief > brief.md
  glasstarn ocel order-management.jsonocel --json
  glasstarn ocel order-management.jsonocel --object-type package --json
  glasstarn ocel order-management.jsonocel --follow order --output orders.csv
  glasstarn ocel training.jsonocel --discover-model --json > baseline.ocelmodel.json
  glasstarn ocel current.jsonocel --conformance-model baseline.ocelmodel.json
  glasstarn discover orders.csv --noise-threshold 20
  glasstarn discover orders.csv --json > model.json
  glasstarn conformance this-month.csv --model approved-process.json
  glasstarn import-bpmn approved-process.bpmn --json > approved-process.json
  glasstarn analyze orders.csv --save-project orders.json
  glasstarn analyze --project orders.json
  glasstarn analyze orders.csv --rename APPROVAL_GRANTED=Approve \\
                                 --rename Approved=Approve \\
                                 --exclude-activity \"System Auto-Update\""
    );
}

fn mcp_command(args: &[String]) -> Result<ExitCode, String> {
    let mut root = ".".to_owned();
    let mut exposure = None;
    let mut live_port = None;
    let mut connection = None;
    let mut project_id = None;
    let mut project_dir_given = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--connection" => connection = Some(take(args, &mut i, "--connection")?),
            "--project-dir" => {
                root = take(args, &mut i, "--project-dir")?;
                project_dir_given = true;
            }
            "--project-id" => {
                let id = take(args, &mut i, "--project-id")?;
                if !pm_project::workspace::valid_workspace_id(&id) {
                    return Err("--project-id expects a project folder identifier".into());
                }
                project_id = Some(id);
            }
            "--exposure-policy" => exposure = Some(take(args, &mut i, "--exposure-policy")?),
            "--live-port" => {
                let value = take(args, &mut i, "--live-port")?;
                let parsed = value.parse::<u16>().map_err(|_| {
                    format!("--live-port expects a number from 1 to 65535, got '{value}'")
                })?;
                if parsed == 0 {
                    return Err("--live-port expects a number from 1 to 65535, got '0'".to_owned());
                }
                live_port = Some(parsed);
            }
            other => return Err(format!("unknown mcp option '{other}'")),
        }
        i += 1;
    }
    if let Some(path) = connection {
        if live_port.is_some() || exposure.is_some() || project_dir_given {
            return Err("--connection cannot be combined with --live-port, --exposure-policy or --project-dir".into());
        }
        let id = project_id
            .as_deref()
            .ok_or("--connection requires --project-id")?;
        pm_mcp::run_connection_stdio(std::path::Path::new(&path), id)?;
        return Ok(ExitCode::SUCCESS);
    }
    if project_id.is_some() && live_port.is_none() {
        return Err("--project-id requires --live-port".into());
    }
    if live_port.is_some() {
        if exposure.is_none() {
            return Err(
                "--live-port requires an explicit --exposure-policy reviewed by the user"
                    .to_owned(),
            );
        }
        if project_dir_given {
            return Err("--live-port delegates to the running application and cannot be combined with --project-dir".to_owned());
        }
    }
    let policy = match exposure.as_deref() {
        Some(path) => pm_mcp::ExposurePolicy::load(path)?,
        None => pm_mcp::ExposurePolicy::safe(),
    };
    if let Some(port) = live_port {
        pm_mcp::run_project_delegated_stdio(port, policy, project_id.as_deref())?;
    } else {
        pm_mcp::run_stdio(root, policy)?;
    }
    Ok(ExitCode::SUCCESS)
}

fn mcp_policy_command(args: &[String]) -> Result<ExitCode, String> {
    let [path] = args else {
        return Err("mcp-policy needs an output file, for example mcp-exposure.json".to_owned());
    };
    let policy = pm_mcp::ExposurePolicy::safe();
    policy.save(path)?;
    println!("Wrote a privacy-safe exposure policy to {path}.");
    println!("Review it before connecting an AI client; the alias key must remain private.");
    Ok(ExitCode::SUCCESS)
}

fn mcp_preview_command(args: &[String]) -> Result<ExitCode, String> {
    let mut project_path = None;
    let mut policy_path = None;
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--exposure-policy" => policy_path = Some(take(args, &mut i, "--exposure-policy")?),
            "--json" => json = true,
            other if other.starts_with('-') => {
                return Err(format!("unknown mcp-preview option '{other}'"));
            }
            other if project_path.replace(other.to_owned()).is_some() => {
                return Err("mcp-preview accepts one saved project".to_owned());
            }
            _ => {}
        }
        i += 1;
    }
    let project_path = project_path.ok_or("mcp-preview needs a saved project JSON file")?;
    let project = Project::load(&project_path).map_err(|error| error.to_string())?;
    let policy = match policy_path {
        Some(path) => pm_mcp::ExposurePolicy::load(path)?,
        None => pm_mcp::ExposurePolicy::safe(),
    };
    let preview = pm_mcp::preview_project(&project, &policy)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&preview).map_err(|error| error.to_string())?
        );
    } else {
        println!("AI EXPOSURE PREVIEW (LOCAL ONLY)\n");
        println!(
            "project  {}  →  {}",
            preview.project_name.local_value,
            preview
                .project_name
                .outbound_value
                .as_deref()
                .unwrap_or("<omitted>")
        );
        println!(
            "source   {}  →  {}\n",
            preview.source_file.local_value,
            preview
                .source_file
                .outbound_value
                .as_deref()
                .unwrap_or("<omitted>")
        );
        for field in &preview.fields {
            println!(
                "{}: {}  →  {}  ({:?})",
                field.role,
                field.local_column,
                field.outbound_column.as_deref().unwrap_or("<omitted>"),
                field.mode
            );
            for example in &field.examples {
                println!(
                    "  {}  →  {}",
                    truncate(&example.local_value, 36),
                    example.outbound_value.as_deref().unwrap_or("<omitted>")
                );
            }
        }
        println!("\n{}", preview.note);
    }
    Ok(ExitCode::SUCCESS)
}

/// `glasstarn open`: starts the browser service in the background, opens the
/// project in the person's own browser and returns at once with its link, so
/// an agent can show the results without keeping a command running. The
/// service keeps running after the agent's command ends.
fn open_command(args: &[String]) -> Result<ExitCode, String> {
    let mut serve_args = vec!["serve".to_owned(), "--no-open".to_owned()];
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--workspace-dir" | "--project" => {
                serve_args.push(args[i].clone());
                serve_args.push(take(args, &mut i, "an option")?);
            }
            "--json" => {}
            other => return Err(format!("unknown open option '{other}'")),
        }
        i += 1;
    }
    // The project already open in a running glasstarn: use that page instead
    // of starting another.
    let show = |url: &str| {
        std::env::var_os("GLASSTARN_NO_BROWSER").is_none() && launch_browser(url).is_ok()
    };
    // The page is running and picks up new answers by itself, so no new tab.
    if let Some((url, showing)) = already_open(&serve_args) {
        // One page per project: while its tab is showing, it updates itself
        // and nothing new opens. If its tab was closed, open it again; it
        // comes back on the same view and filters.
        let opened = !showing && show(&url);
        let result = serde_json::json!({"status": "open", "url": url, "opened_in_browser": opened, "already_running": true, "page_showing": showing, "update_available": update::available()});
        println!(
            "{}",
            serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }
    let log_path = std::env::temp_dir().join(format!(
        "glasstarn-open-{}-{}.log",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let log = std::fs::File::create(&log_path).map_err(|e| e.to_string())?;
    let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    command
        .args(&serve_args)
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group, so it outlives the agent's command.
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start glasstarn: {e}"))?;
    let started = std::time::Instant::now();
    loop {
        let text = std::fs::read_to_string(&log_path).unwrap_or_default();
        if let Some(url) = text
            .lines()
            .find_map(|line| line.strip_prefix("open "))
            .map(str::to_owned)
        {
            // A plain address: the page needs no key, so the link works
            // wherever it is clicked or pasted.
            let (base, rest) = url.split_once("/#").unwrap_or((url.as_str(), ""));
            let project = rest
                .split('&')
                .find_map(|part| part.strip_prefix("project="))
                .map(|id| format!("?project={id}"))
                .unwrap_or_default();
            let url = format!("{base}/{project}");
            let opened = show(&url);
            let result = serde_json::json!({
                "status": "open",
                "url": url,
                "opened_in_browser": opened,
                "note": "glasstarn keeps running in the background so this link works; give the person this link if their browser did not open.",
                "update_available": update::available()
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?
            );
            return Ok(ExitCode::SUCCESS);
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "glasstarn could not start the browser view ({status}): {}",
                text.trim()
            ));
        }
        if started.elapsed() > std::time::Duration::from_secs(20) {
            return Err("glasstarn did not start the browser view in time".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// The page of a running glasstarn that has the named project open, read from
/// the note it leaves in the project folder, when that page still answers;
/// and whether a browser tab is still showing it.
fn already_open(serve_args: &[String]) -> Option<(String, bool)> {
    let value = |name: &str| {
        serve_args
            .iter()
            .position(|arg| arg == name)
            .and_then(|i| serve_args.get(i + 1))
    };
    let id = value("--project")?;
    let root = match value("--workspace-dir") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::path::PathBuf::from(std::env::var_os("HOME")?)
            .join(".glasstarn")
            .join("projects"),
    };
    let note_path = root.join(id).join(".browser-session.json");
    let text = std::fs::read_to_string(&note_path).ok()?;
    let note = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    let url = note["url"].as_str()?.to_owned();
    let address: std::net::SocketAddr = url
        .strip_prefix("http://")?
        .trim_end_matches('/')
        .parse()
        .ok()?;
    if !address.ip().is_loopback() {
        return None;
    }
    let answers = || {
        std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(300))
            .is_ok()
    };
    if !answers() {
        return None;
    }
    // A page left running by an older glasstarn would show the old screens:
    // stop it, and let this build start the project afresh.
    if note["build"].as_str() != Some(pm_server::build_id()) {
        // The process actually answering on that port, so a recorded number
        // reused by another program is never stopped.
        let pid = if cfg!(unix) {
            listening_process(address.port())
        } else {
            note["pid"].as_u64().map(|pid| pid.to_string())
        };
        if let Some(pid) = pid {
            stop_process(&pid);
            for _ in 0..50 {
                if !answers() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        let _ = std::fs::remove_file(&note_path);
        return None;
    }
    Some((url, page_showing(address)))
}

/// Whether a browser tab still shows this page: an open page checks in every
/// few seconds.
fn page_showing(address: std::net::SocketAddr) -> bool {
    use std::io::{Read, Write};
    let Ok(mut stream) =
        std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(300))
    else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
    let request = format!(
        "GET /api/health HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    response
        .split_once("\r\n\r\n")
        .and_then(|(_, body)| serde_json::from_str::<serde_json::Value>(body).ok())
        .and_then(|health| health["page_seen_seconds_ago"].as_u64())
        .is_some_and(|seconds| seconds <= 20)
}

/// The glasstarn process listening on a local port, if it is one.
fn listening_process(port: u16) -> Option<String> {
    #[cfg(unix)]
    {
        let out = Command::new("lsof")
            .args(["-nP", "-t", &format!("-iTCP:{port}"), "-sTCP:LISTEN"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let pid = text.lines().next()?.trim().to_owned();
        // Only ever a glasstarn: never stop some other program on that port.
        let name = Command::new("ps")
            .args(["-p", &pid, "-o", "comm="])
            .output()
            .ok()?;
        String::from_utf8_lossy(&name.stdout)
            .contains("glasstarn")
            .then_some(pid)
    }
    #[cfg(not(unix))]
    {
        let _ = port;
        None
    }
}

fn stop_process(pid: &str) {
    #[cfg(unix)]
    let _ = Command::new("kill").arg(pid).status();
    #[cfg(windows)]
    let _ = Command::new("taskkill").args(["/PID", pid, "/F"]).status();
}

fn serve_command(args: &[String]) -> Result<ExitCode, String> {
    // Fix which build this process runs before anything can be reinstalled.
    let _ = pm_server::build_id();
    let mut port = 0u16;
    let mut open = true;
    let mut agent_access = None;
    let mut workspace_dir = None;
    let mut project = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                let value = take(args, &mut i, "--port")?;
                port = value.parse().map_err(|_| {
                    format!("--port expects a number from 0 to 65535, got '{value}'")
                })?;
            }
            "--workspace-dir" => workspace_dir = Some(take(args, &mut i, "--workspace-dir")?),
            "--project" => project = Some(take(args, &mut i, "--project")?),
            "--no-open" => open = false,
            "--agent-access" => agent_access = Some(take(args, &mut i, "--agent-access")?),
            other => return Err(format!("unknown serve option '{other}'")),
        }
        i += 1;
    }

    if agent_access.is_some() && port == 0 {
        port = pm_mcp::DEFAULT_LIVE_PORT;
    }
    let agent_policy = match agent_access.as_deref() {
        Some(path) => Some(pm_mcp::ExposurePolicy::load(path)?),
        None => None,
    };
    let workspace_root = match workspace_dir {
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            if path.is_dir() {
                pm_project::folders::remember_root(&path);
            }
            path
        }
        None => std::path::PathBuf::from(
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .ok_or("cannot find your home folder; pass --workspace-dir")?,
        )
        .join(".glasstarn")
        .join("projects"),
    };
    if let Some(id) = project.as_deref() {
        if !pm_project::workspace::valid_workspace_id(id) {
            return Err(
                "--project expects the project identifier returned by workspace create/show".into(),
            );
        }
        let manifest = pm_project::workspace::WorkspaceFolder::inspect(&workspace_root.join(id))
            .map_err(|error| format!("cannot open requested project: {error}"))?;
        if manifest.id != id {
            return Err("project folder identity does not match --project".into());
        }
    }
    let server = pm_server::Server::bind_with_workspaces(port, agent_policy, &workspace_root)?;
    println!("project folders: {}", workspace_root.display());
    let mut launch_url = server.launch_url();
    if let Some(id) = project {
        launch_url.push_str(&format!("&project={id}"));
    }
    println!("local service listening on {}", server.address());
    println!("open {launch_url}");
    if let Some(path) = agent_access.as_deref() {
        println!(
            "live agent access enabled on {} with reviewed policy {path}",
            server.address()
        );
    }
    if open {
        match launch_browser(&launch_url) {
            Ok(()) => println!("opened the local application in your browser"),
            Err(error) => eprintln!("could not open a browser automatically: {error}"),
        }
    }
    // A page nobody uses stops after two hours (GLASSTARN_IDLE_MINUTES, 0 for
    // never), so pages from past days do not pile up in the background. The
    // agent or `glasstarn` starts a fresh one when it is needed.
    pm_server::set_update_available(update::available());
    let stop = Arc::new(AtomicBool::new(false));
    let idle_limit = std::env::var("GLASSTARN_IDLE_SECONDS")
        .ok()
        .and_then(|seconds| seconds.parse::<u64>().ok())
        .unwrap_or_else(|| {
            std::env::var("GLASSTARN_IDLE_MINUTES")
                .ok()
                .and_then(|minutes| minutes.parse::<u64>().ok())
                .unwrap_or(120)
                * 60
        });
    if idle_limit > 0 {
        let stop = Arc::clone(&stop);
        pm_server::idle_seconds();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(idle_limit.clamp(1, 60)));
                if pm_server::idle_seconds() >= idle_limit {
                    stop.store(true, std::sync::atomic::Ordering::Release);
                    break;
                }
            }
        });
    }
    server.run(stop).map_err(|error| error.to_string())?;
    Ok(ExitCode::SUCCESS)
}

#[cfg(target_os = "macos")]
fn browser_command(url: &str) -> (&'static str, Vec<String>) {
    ("open", vec![url.to_owned()])
}

#[cfg(target_os = "windows")]
fn browser_command(url: &str) -> (&'static str, Vec<String>) {
    (
        "rundll32",
        vec!["url.dll,FileProtocolHandler".to_owned(), url.to_owned()],
    )
}

#[cfg(all(unix, not(target_os = "macos")))]
fn browser_command(url: &str) -> (&'static str, Vec<String>) {
    ("xdg-open", vec![url.to_owned()])
}

fn launch_browser(url: &str) -> Result<(), String> {
    let (program, args) = browser_command(url);
    Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("{program} failed: {error}; open the printed URL manually"))
}

fn formats_command(args: &[String]) -> Result<ExitCode, String> {
    let json = match args {
        [] => false,
        [flag] if flag == "--json" => true,
        [flag] => return Err(format!("unknown formats option '{flag}'")),
        _ => return Err("formats accepts only --json".to_owned()),
    };
    if json {
        let formats: Vec<_> = pm_ingest::timestamp::all()
            .iter()
            .map(|format| {
                serde_json::json!({
                    "id": format.id,
                    "pattern": format.pattern,
                    "example": format.example,
                    "meaning": format.meaning,
                    "shape": format_shape(format.shape),
                    "order": format_order(format.order),
                    "zone": format_zone(format.zone),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&formats).map_err(|error| error.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }
    println!("Recognised timestamp formats:\n");
    for f in pm_ingest::timestamp::all() {
        println!("  {:<28} {:<32} {}", f.id, f.example, f.meaning);
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, PartialEq, Eq)]
struct OcelArgs {
    path: String,
    filter: OcelFilter,
    json: bool,
    bottlenecks: bool,
    project_as: Option<String>,
    discover_model: bool,
    conformance_model: Option<String>,
    conformance_alignment: bool,
    discovery_config: pm_core::OcelDiscoveryConfig,
    case_limit: usize,
    event_limit: usize,
    detail_limit: usize,
}

fn parse_ocel_args(args: &[String]) -> Result<OcelArgs, String> {
    let mut path = None;
    let mut filter = OcelFilter::default();
    let mut json = false;
    let mut bottlenecks = false;
    let mut project_as = None;
    let mut discover_model = false;
    let mut conformance_model = None;
    let mut conformance_alignment = false;
    let mut conformance_algorithm_explicit = false;
    let mut discovery_config = pm_core::OcelDiscoveryConfig::default();
    let mut discovery_config_explicit = false;
    let mut case_limit = 5usize;
    let mut event_limit = 100usize;
    let mut detail_limit = 100usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json = true,
            "--bottlenecks" => bottlenecks = true,
            "--project-as" => project_as = Some(take(args, &mut i, "--project-as")?),
            "--discover-model" => discover_model = true,
            "--conformance-model" => {
                conformance_model = Some(take(args, &mut i, "--conformance-model")?)
            }
            "--conformance-algorithm" => {
                let value = take(args, &mut i, "--conformance-algorithm")?;
                conformance_alignment = match value.as_str() {
                    "token" => false,
                    "alignment" => true,
                    _ => {
                        return Err(format!(
                            "--conformance-algorithm expects 'token' or 'alignment', got '{value}'"
                        ));
                    }
                };
                conformance_algorithm_explicit = true;
            }
            "--noise-threshold" => {
                discovery_config.noise_threshold_milli = parse_percent_threshold(
                    &take(args, &mut i, "--noise-threshold")?,
                    "--noise-threshold",
                )?;
                discovery_config_explicit = true;
            }
            "--variable-arc-threshold" => {
                discovery_config.variable_arc_threshold_milli = parse_percent_threshold(
                    &take(args, &mut i, "--variable-arc-threshold")?,
                    "--variable-arc-threshold",
                )?;
                discovery_config_explicit = true;
            }
            "--case-limit" => {
                case_limit =
                    parse_bounded_usize(&take(args, &mut i, "--case-limit")?, "--case-limit", 100)?;
            }
            "--event-limit" => {
                event_limit = parse_bounded_usize(
                    &take(args, &mut i, "--event-limit")?,
                    "--event-limit",
                    1_000,
                )?;
            }
            "--detail-limit" => {
                detail_limit = parse_bounded_usize(
                    &take(args, &mut i, "--detail-limit")?,
                    "--detail-limit",
                    10_000,
                )?;
            }
            "--object-type" => filter
                .object_types
                .push(take(args, &mut i, "--object-type")?),
            "--e2o-qualifier" => {
                filter
                    .event_object_qualifiers
                    .push(take(args, &mut i, "--e2o-qualifier")?)
            }
            "--o2o-qualifier" => {
                filter
                    .object_object_qualifiers
                    .push(take(args, &mut i, "--o2o-qualifier")?)
            }
            option if option.starts_with('-') => {
                return Err(format!("unknown ocel option '{option}'"));
            }
            source => {
                if path.replace(source.to_owned()).is_some() {
                    return Err("ocel accepts only one source file".to_owned());
                }
            }
        }
        i += 1;
    }
    if usize::from(bottlenecks)
        + usize::from(project_as.is_some())
        + usize::from(discover_model)
        + usize::from(conformance_model.is_some())
        > 1
    {
        return Err(
            "choose only one of --bottlenecks, --project-as, --discover-model, or --conformance-model"
                .to_owned(),
        );
    }
    if project_as.is_none() && (case_limit != 5 || event_limit != 100) {
        return Err("--case-limit and --event-limit require --project-as".to_owned());
    }
    if discovery_config_explicit && !discover_model {
        return Err(
            "--noise-threshold and --variable-arc-threshold require --discover-model".to_owned(),
        );
    }
    if conformance_model.is_none() && detail_limit != 100 {
        return Err("--detail-limit requires --conformance-model".to_owned());
    }
    if conformance_model.is_none() && conformance_algorithm_explicit {
        return Err("--conformance-algorithm requires --conformance-model".to_owned());
    }
    Ok(OcelArgs {
        path: path.ok_or("ocel needs an OCEL 2.0 JSON, XML, or SQLite file")?,
        filter,
        json,
        bottlenecks,
        project_as,
        discover_model,
        conformance_model,
        conformance_alignment,
        discovery_config,
        case_limit,
        event_limit,
        detail_limit,
    })
}

fn parse_bounded_usize(value: &str, option: &str, maximum: usize) -> Result<usize, String> {
    let value = value
        .parse::<usize>()
        .map_err(|_| format!("{option} expects a positive integer"))?;
    if !(1..=maximum).contains(&value) {
        return Err(format!("{option} must be between 1 and {maximum}"));
    }
    Ok(value)
}

fn ocel_command(args: &[String]) -> Result<ExitCode, String> {
    // `--follow <type> --output <file.csv>` writes the case log of one item
    // type, for any case tool or project.
    let mut rest = Vec::new();
    let (mut follow, mut output) = (None, None);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--follow" => follow = Some(take(args, &mut i, "--follow")?),
            "--output" => output = Some(take(args, &mut i, "--output")?),
            _ => rest.push(args[i].clone()),
        }
        i += 1;
    }
    let args = parse_ocel_args(&rest)?;
    if let Some(object_type) = follow {
        let output = output.ok_or("--follow needs --output <file.csv>")?;
        let file = std::fs::File::create(&output)
            .map_err(|error| format!("cannot write {output}: {error}"))?;
        let summary = pm_query::follow_ocel(
            &args.path,
            &args.filter,
            &object_type,
            std::io::BufWriter::new(file),
        )
        .map_err(|error| error.to_string())?;
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).map_err(|error| error.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }
    if output.is_some() {
        return Err("--output is used with --follow <item type>".into());
    }
    if args.bottlenecks {
        let report = pm_query::rank_ocel_bottlenecks(&args.path, &args.filter)
            .map_err(|error| error.to_string())?;
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
            );
        } else {
            print_ocel_bottlenecks(&report);
        }
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(case_object_type) = &args.project_as {
        let report = pm_query::project_ocel_cases(
            &args.path,
            &args.filter,
            case_object_type,
            args.case_limit,
            args.event_limit,
        )
        .map_err(|error| error.to_string())?;
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
            );
        } else {
            print_ocel_projection(&report);
        }
        return Ok(ExitCode::SUCCESS);
    }
    if args.discover_model {
        let report = pm_query::discover_ocel_model(&args.path, &args.filter, args.discovery_config)
            .map_err(|error| error.to_string())?;
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
            );
        } else {
            print_ocel_model(&report);
        }
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(model) = &args.conformance_model {
        if args.conformance_alignment {
            let report = pm_query::align_ocel_conformance(
                &args.path,
                model,
                &args.filter,
                args.detail_limit,
            )
            .map_err(|error| error.to_string())?;
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
                );
            } else {
                print_ocel_alignment(&report);
            }
        } else {
            let report = pm_query::check_ocel_conformance(
                &args.path,
                model,
                &args.filter,
                args.detail_limit,
            )
            .map_err(|error| error.to_string())?;
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
                );
            } else {
                print_ocel_conformance(&report);
            }
        }
        return Ok(ExitCode::SUCCESS);
    }
    let analysis =
        pm_query::analyze_ocel(&args.path, &args.filter).map_err(|error| error.to_string())?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&analysis).map_err(|error| error.to_string())?
        );
        return Ok(if analysis.ready_for_analysis {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(3)
        });
    }

    println!("OBJECT-CENTRIC ANALYSIS");
    println!("  source:  {}", analysis.source);
    println!(
        "  scope:   {} of {} events · {} of {} objects",
        analysis.scope.events_shown,
        analysis.scope.events_total,
        analysis.scope.objects_shown,
        analysis.scope.objects_total
    );
    if analysis.scope.narrowed {
        println!("  view:    filtered (all counts below use this scope)");
    }
    println!(
        "  quality: {} error findings · {} warnings · {} information",
        analysis.quality.summary.errors,
        analysis.quality.summary.warnings,
        analysis.quality.summary.information
    );
    if !analysis.ready_for_analysis {
        println!("\nAnalysis is blocked until the quality errors are resolved:");
        for finding in &analysis.quality.findings {
            if finding.severity == pm_ingest::OcelQualitySeverity::Error {
                println!("  • {} ({})", finding.title, finding.affected);
            }
        }
        return Ok(ExitCode::from(3));
    }
    let graphs = analysis.graphs.as_ref().expect("ready analysis has graphs");
    println!("\nACTIVITIES");
    for activity in &graphs.activities {
        println!("  {:>10}  {}", activity.events, activity.activity);
    }
    println!("\nOBJECT LIFECYCLE FLOW");
    for edge in &graphs.directly_follows {
        println!(
            "  {:>10}  [{}] {} → {} ({} objects)",
            edge.occurrences, edge.object_type, edge.source, edge.target, edge.objects
        );
    }
    println!("\nOBJECT-TYPE INTERACTIONS");
    for edge in &graphs.interactions {
        println!(
            "  {:>10}  {} ↔ {} ({} object pairs)",
            edge.events, edge.left_object_type, edge.right_object_type, edge.object_pairs
        );
    }
    println!("\nUse --json for qualified participation, explicit relationships, and evidence.");
    Ok(ExitCode::SUCCESS)
}

fn print_ocel_bottlenecks(report: &pm_query::OcelBottleneckReport) {
    println!("OBJECT-CENTRIC TIMING PRIORITIES");
    println!("  source: {}", report.source);
    println!(
        "  lifecycle: {} positive of {} candidates · {} total elapsed",
        report.lifecycle_positive_candidates,
        report.lifecycle_candidates,
        duration(report.total_lifecycle_elapsed_micros / 1_000_000)
    );
    println!("\nLIFECYCLE ELAPSED BURDEN");
    if report.lifecycle.is_empty() {
        println!("  No lifecycle transition has positive elapsed burden.");
    }
    for item in &report.lifecycle {
        println!(
            "  #{:<3} {:>8.2}%  {:>8}  [{}] {} → {} ({} {}, {} {})",
            item.rank,
            item.share_millionths as f64 / 10_000.0,
            duration(item.total_elapsed_micros / 1_000_000),
            item.object_type,
            item.source,
            item.target,
            item.occurrences,
            if item.occurrences == 1 {
                "occurrence"
            } else {
                "occurrences"
            },
            item.objects,
            if item.objects == 1 {
                "object"
            } else {
                "objects"
            }
        );
    }
    println!(
        "\nCONVERGENCE SPREAD BURDEN ({} positive of {} eligible activities)",
        report.synchronization_positive_activities, report.synchronization_eligible_activities
    );
    if report.synchronization.is_empty() {
        println!("  No eligible activity has positive prior-point spread.");
    }
    for item in &report.synchronization {
        println!(
            "  #{:<3} {:>8.2}%  {:>8}  {} ({} eligible {})",
            item.rank,
            item.share_millionths as f64 / 10_000.0,
            duration(item.total_arrival_spread_micros / 1_000_000),
            item.activity,
            item.eligible_events,
            if item.eligible_events == 1 {
                "event"
            } else {
                "events"
            }
        );
    }
    println!(
        "\nThese are separate rankings of observed timing burden, not waiting or causal diagnoses. Use --json for bounded source evidence and exact microsecond metrics."
    );
}

fn print_ocel_projection(report: &pm_query::OcelProjectionReport) {
    println!("EXPLICIT CASE-CENTRIC PROJECTION");
    println!("  source:    {}", report.source);
    println!("  case type: {}", report.case_object_type);
    println!("  cases:     {}", report.projected_cases);
    println!(
        "  events:    {} of {} unique source events included · {} projected occurrences",
        report.included_unique_events, report.source_events, report.projected_event_occurrences
    );
    if report.excluded_events > 0 {
        println!(
            "  excluded:  {} source events have no retained {} object",
            report.excluded_events, report.case_object_type
        );
    }
    println!("\nDISTORTION WARNINGS");
    if report.convergence_events == 0 {
        println!("  Convergence: none observed.");
    } else {
        println!(
            "  Convergence: {} source events are copied into several cases, adding {} occurrences.",
            report.convergence_events, report.convergence_additional_occurrences
        );
    }
    if report.divergence_candidates == 0 {
        println!("  Divergence: no evidence-backed candidates observed.");
    } else {
        println!(
            "  Divergence: {} repeated-activity candidates involve distinct objects of another type.",
            report.divergence_candidates
        );
        println!("  These are risks to investigate, not proof of artificial loops.");
    }
    println!(
        "\nPROJECTED CASE PREVIEW ({} of {})",
        report.cases_returned, report.cases_available
    );
    for case in &report.cases {
        let path = case
            .layers
            .iter()
            .map(|layer| {
                layer
                    .events
                    .iter()
                    .map(|event| event.activity.as_str())
                    .collect::<Vec<_>>()
                    .join(" + ")
            })
            .collect::<Vec<_>>()
            .join(" → ");
        println!(
            "  {}  {}{}",
            case.case_object,
            path,
            if case.truncated {
                "  … (truncated)"
            } else {
                ""
            }
        );
    }
    println!(
        "\nProjection is explicit and auditable. Events joined to several selected objects are copied; simultaneous events joined with '+' are unordered. Use --json for source identifiers, exact timestamps, and bounded warning evidence."
    );
}

fn print_ocel_model(report: &pm_query::OcelModelReport) {
    let visible = report
        .transitions
        .iter()
        .filter(|transition| transition.activity.is_some())
        .count();
    let silent = report.transitions.len().saturating_sub(visible);
    println!("NATIVE OBJECT-CENTRIC PETRI NET");
    println!("  source:      {}", report.source);
    println!(
        "  model:       {} places · {} visible transitions · {} silent transitions",
        report.places.len(),
        visible,
        silent
    );
    println!(
        "  arcs:        {} fixed · {} variable",
        report.fixed_arcs, report.variable_arcs
    );
    println!(
        "  thresholds:  {}% noise · {}% fixed-arc confidence",
        format_noise_threshold(report.noise_threshold_milli),
        format_noise_threshold(report.variable_arc_threshold_milli)
    );
    println!("\nOBJECT TYPES");
    for stats in &report.object_types {
        println!(
            "  {:<20} {} of {} objects with events · {} lifecycle occurrences{}{}",
            stats.object_type,
            stats.objects_with_events,
            stats.objects,
            stats.lifecycle_event_occurrences,
            if stats.filtered_edges > 0 {
                " · infrequent edges filtered"
            } else {
                ""
            },
            if stats.used_flower_fallback {
                " · permissive fallback used"
            } else {
                ""
            }
        );
    }
    println!(
        "\nShared visible activities connect typed lifecycle nets. Variable arcs mark observed multiplicity; silent transitions route tokens. Use --json for every place, transition, and arc."
    );
}

fn print_ocel_conformance(report: &pm_query::OcelConformanceReport) {
    println!("OBJECT-CENTRIC CONFORMANCE");
    println!("  tested log:     {}", report.source);
    println!("  baseline model: {}", report.model_source);
    println!(
        "  token fitness:  {:.2}%",
        report.fitness_millionths as f64 / 10_000.0
    );
    println!(
        "  object paths:   {} of {} lifecycle-fit",
        report.lifecycle_fit_objects, report.replayed_objects
    );
    println!(
        "  source events:  {} of {} fit · {} outside active modeled types",
        report.fit_events, report.replayed_events, report.out_of_scope_events
    );
    println!(
        "  token issues:   {} missing · {} remaining · {} unmapped occurrences",
        report.missing_tokens, report.remaining_tokens, report.unmapped_event_occurrences
    );
    println!(
        "  binding issues: {} shared-event participation or multiplicity violations",
        report.binding_violations
    );

    if !report.objects.is_empty() {
        println!("\nWORST OBJECT LIFECYCLES");
        for object in &report.objects {
            println!(
                "  {:<24} {:<16} {:>6.2}% · {} events{}",
                object.object,
                object.object_type,
                object.fitness_millionths as f64 / 10_000.0,
                object.events,
                if object.lifecycle_fit {
                    ""
                } else {
                    " · deviates"
                }
            );
        }
        if report.objects_truncated {
            println!(
                "  … {} more objects (totals above remain complete)",
                report.objects_available - report.objects_returned
            );
        }
    }

    if !report.deviations.is_empty() {
        println!("\nDEVIATIONS");
        for deviation in &report.deviations {
            println!("  {}", format_ocel_deviation(deviation));
        }
        if report.deviations_truncated {
            println!(
                "  … {} more deviations (totals above remain complete)",
                report.deviations_available - report.deviations_returned
            );
        }
    }
    println!(
        "\nLifecycle replay and shared-event binding are distinct checks. This is diagnostic evidence, not proof of regulatory compliance. Use --json for complete machine-readable fields and bounded source evidence."
    );
}

fn print_ocel_alignment(report: &pm_query::OcelAlignmentConformanceReport) {
    println!("OBJECT-CENTRIC ALIGNMENT CONFORMANCE");
    println!("  tested log:       {}", report.source);
    println!("  baseline model:   {}", report.model_source);
    println!(
        "  alignment fitness: {:.2}% · total cost {}",
        report.fitness_millionths as f64 / 10_000.0,
        report.total_cost
    );
    println!(
        "  object paths:     {} of {} zero-cost",
        report.lifecycle_fit_objects, report.aligned_objects
    );
    println!(
        "  source events:    {} of {} fit · {} outside active modeled types",
        report.fit_events, report.aligned_events, report.out_of_scope_events
    );
    println!(
        "  binding issues:   {} shared-event participation or multiplicity violations",
        report.binding_violations
    );

    if !report.objects.is_empty() {
        println!("\nWORST OBJECT ALIGNMENTS");
        for object in &report.objects {
            println!(
                "  {:<24} {:<16} cost {:>4} · {:>6.2}% · {} log / {} model moves",
                object.object,
                object.object_type,
                object.cost,
                object.fitness_millionths as f64 / 10_000.0,
                object.log_moves,
                object.visible_model_moves
            );
            for deviation in &object.deviations {
                match deviation {
                    pm_query::LabelledOcelAlignmentDeviation::LogOnly { event, activity } => {
                        println!("    observed only: event {event} '{activity}'");
                    }
                    pm_query::LabelledOcelAlignmentDeviation::VisibleModel {
                        transition,
                        activity,
                    } => {
                        println!("    model only: transition {transition} '{activity}'");
                    }
                }
            }
            if object.deviations_truncated {
                println!(
                    "    … {} more lifecycle deviations",
                    object.deviations_available - object.deviations_returned
                );
            }
        }
        if report.objects_truncated {
            println!(
                "  … {} more objects (totals above remain complete)",
                report.objects_available - report.objects_returned
            );
        }
    }
    if !report.binding_deviations.is_empty() {
        println!("\nBINDING DEVIATIONS");
        for deviation in &report.binding_deviations {
            println!("  {}", format_ocel_deviation(deviation));
        }
    }
    println!(
        "\nAlignment cost covers lifecycle control flow. Shared-event binding is reported separately. This is diagnostic evidence, not proof of regulatory compliance."
    );
}

fn format_ocel_deviation(deviation: &pm_query::LabelledOcelConformanceDeviation) -> String {
    use pm_query::LabelledOcelConformanceDeviation as Deviation;
    match deviation {
        Deviation::ActivityNotInModel {
            event,
            activity,
            object,
        } => format!("event {event}: activity '{activity}' is not in the model for {object}"),
        Deviation::MissingTokens {
            event,
            activity,
            object,
            count,
        } => format!("event {event}: '{activity}' needed {count} missing token(s) for {object}"),
        Deviation::FinalTokensMissing { object, count } => {
            format!("object {object}: {count} final token(s) missing")
        }
        Deviation::TokensRemaining { object, count } => {
            format!("object {object}: {count} token(s) remained outside the final place")
        }
        Deviation::MissingObjectParticipation {
            event,
            activity,
            object_type,
            variable,
        } => format!(
            "event {event}: '{}' is missing {} participation{}",
            activity.as_deref().unwrap_or("silent transition"),
            object_type,
            if *variable { " on a variable arc" } else { "" }
        ),
        Deviation::FixedArcMultiplicity {
            event,
            activity,
            object_type,
            observed_objects,
        } => format!(
            "event {event}: '{}' expected exactly one {object_type} object but observed {observed_objects}",
            activity.as_deref().unwrap_or("silent transition")
        ),
        Deviation::UnexpectedObjectParticipation {
            event,
            activity,
            object_type,
            observed_objects,
        } => format!(
            "event {event}: '{}' unexpectedly involved {observed_objects} {object_type} object(s)",
            activity.as_deref().unwrap_or("silent transition")
        ),
    }
}

fn format_shape(shape: pm_ingest::timestamp::Shape) -> &'static str {
    use pm_ingest::timestamp::Shape;
    match shape {
        Shape::DateTime => "date_time",
        Shape::DateOnly => "date_only",
        Shape::EpochSeconds => "epoch_seconds",
        Shape::EpochMillis => "epoch_milliseconds",
        Shape::EpochMicros => "epoch_microseconds",
    }
}

fn format_order(order: pm_ingest::timestamp::Order) -> &'static str {
    use pm_ingest::timestamp::Order;
    match order {
        Order::YearFirst => "year_first",
        Order::DayFirst => "day_first",
        Order::MonthFirst => "month_first",
        Order::Named => "named_month",
        Order::NotApplicable => "not_applicable",
    }
}

fn format_zone(zone: pm_ingest::timestamp::Zone) -> &'static str {
    use pm_ingest::timestamp::Zone;
    match zone {
        Zone::Naive => "naive",
        Zone::Utc => "utc",
        Zone::Offset => "offset",
        Zone::EpochUtc => "epoch_utc",
    }
}

fn quality_command(args: &[String]) -> Result<ExitCode, String> {
    let mut path: Option<String> = None;
    let mut prep = Prep::default();
    let (mut case, mut activity, mut timestamp) = (None, None, None);
    let mut json = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json = true,
            "--case" => case = Some(take(args, &mut i, "--case")?),
            "--activity" => activity = Some(take(args, &mut i, "--activity")?),
            "--timestamp" => timestamp = Some(take(args, &mut i, "--timestamp")?),
            flag @ ("--answer" | "--project") => {
                if !parse_prep_flag(
                    flag,
                    args,
                    &mut i,
                    &mut prep,
                    &mut case,
                    &mut activity,
                    &mut timestamp,
                )? {
                    unreachable!("matched preparation flag")
                }
            }
            other if other.starts_with('-') => return Err(format!("unknown option '{other}'")),
            other => {
                if path.replace(other.to_owned()).is_some() {
                    return Err("quality accepts only one source file".to_owned());
                }
            }
        }
        i += 1;
    }

    let path = path
        .or_else(|| prep.project_source.clone())
        .ok_or("quality needs a file or --project")?;
    let mapping = resolve_mapping(&prep, case, activity, timestamp)?;
    let quality =
        pm_query::assess_quality(&path, mapping, &prep.policy).map_err(|e| e.to_string())?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&quality).map_err(|e| e.to_string())?
        );
    } else {
        print_quality(&quality);
    }

    Ok(if quality.ready_for_analysis {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(3)
    })
}

fn print_quality(quality: &pm_query::DataQualityReport) {
    println!("{}", quality.source);
    println!(
        "DATA QUALITY — {}",
        if quality.ready_for_analysis {
            "READY FOR ANALYSIS"
        } else {
            "DECISIONS REQUIRED"
        }
    );
    if quality.findings.is_empty() {
        println!("No data-quality findings.");
        return;
    }
    println!(
        "{} · {} · {}",
        plural(quality.summary.warnings, "warning", "warnings"),
        plural(
            quality.summary.open_decisions,
            "open decision",
            "open decisions"
        ),
        plural(
            quality.summary.information,
            "information item",
            "information items"
        )
    );

    for finding in &quality.findings {
        let level = match finding.severity {
            pm_query::QualitySeverity::Information => "INFO",
            pm_query::QualitySeverity::Warning => "WARNING",
            pm_query::QualitySeverity::Decision => "DECISION",
        };
        println!("\n{level}  {}", finding.title);
        println!("  {}", finding.detail);
        if finding.affected_rows > 0 || finding.affected_cases > 0 {
            println!(
                "  {} affected · {} affected",
                plural(finding.affected_rows, "row", "rows"),
                plural(finding.affected_cases, "case", "cases")
            );
        }
        for example in finding.examples.iter().take(5) {
            println!("    {example}");
        }
        if let Some(decision) = &finding.decision {
            if let Some(answer) = &decision.answer {
                println!(
                    "  answer: {answer}{}",
                    if decision.answered { "" } else { " (invalid)" }
                );
            } else if decision.required {
                println!("  {}", decision.question);
                for option in &decision.options {
                    println!(
                        "    --answer {}={}  {}",
                        kind_flag(&finding.kind),
                        option.id,
                        option.label
                    );
                }
            }
        }
    }
}

/// The settings that decide how a file is read and interpreted.
///
/// Shared by every command, because evidence produced under different settings
/// than the analysis would not be evidence for it.
#[derive(Default)]
struct Prep {
    mapping: Option<Mapping>,
    policy: Policy,
    rules: Rules,
    project_source: Option<String>,
    project_filter: Option<Filter>,
}

/// Parses a preparation flag, returning whether it consumed one.
///
/// Kept in one place so `analyze`, `case` and `cases` cannot drift into
/// reading the same file differently.
fn parse_prep_flag(
    flag: &str,
    args: &[String],
    i: &mut usize,
    prep: &mut Prep,
    case: &mut Option<String>,
    activity: &mut Option<String>,
    timestamp: &mut Option<String>,
) -> Result<bool, String> {
    match flag {
        "--case" => *case = Some(take(args, i, "--case")?),
        "--activity-column" => *activity = Some(take(args, i, "--activity-column")?),
        "--timestamp" => *timestamp = Some(take(args, i, "--timestamp")?),
        "--answer" => {
            let value = take(args, i, "--answer")?;
            let (kind, option) = value
                .split_once('=')
                .ok_or_else(|| format!("--answer expects <kind>=<option>, got '{value}'"))?;
            prep.policy = std::mem::take(&mut prep.policy).answer(parse_kind(kind)?, option);
        }
        "--rename" => {
            let value = take(args, i, "--rename")?;
            let (from, to) = value
                .split_once('=')
                .ok_or_else(|| format!("--rename expects <old>=<new>, got '{value}'"))?;
            prep.rules = std::mem::take(&mut prep.rules).rename(from.trim(), to.trim());
        }
        "--exclude-activity" => {
            let v = take(args, i, "--exclude-activity")?;
            prep.rules = std::mem::take(&mut prep.rules).exclude_activity(v.trim());
        }
        "--exclude-case" => {
            let v = take(args, i, "--exclude-case")?;
            prep.rules = std::mem::take(&mut prep.rules).exclude_case(v.trim());
        }
        "--min-events" => {
            let v = take(args, i, "--min-events")?;
            let n = v
                .parse()
                .map_err(|_| format!("--min-events expects a number, got '{v}'"))?;
            prep.rules = std::mem::take(&mut prep.rules).min_events_per_case(n);
        }
        "--drop-incomplete" => {
            prep.rules = std::mem::take(&mut prep.rules).drop_incomplete_cases(true);
        }
        "--rules" => {
            let path = take(args, i, "--rules")?;
            let loaded = Rules::from_file(&path).map_err(|e| e.to_string())?;
            prep.rules = merge_rules(loaded, std::mem::take(&mut prep.rules));
        }
        "--project" => {
            let path = take(args, i, "--project")?;
            let project = Project::load(&path).map_err(|e| e.to_string())?;
            prep.policy = merge_policies(project.policy.clone(), std::mem::take(&mut prep.policy));
            prep.rules = merge_rules(project.rules.clone(), std::mem::take(&mut prep.rules));
            prep.mapping = Some(project.mapping.clone());
            prep.project_source = Some(project.source.path.clone());
            prep.project_filter = Some(project.view_filter.clone());
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Restores a project's saved view unless this invocation supplied view flags.
fn resolve_filter(prep: &Prep, explicit: bool, filter: Filter) -> Filter {
    if explicit {
        filter
    } else {
        prep.project_filter.clone().unwrap_or(filter)
    }
}

/// Reads one `--where` condition: `region=EMEA` matches a value exactly;
/// `amount>=50000` and `amount<=100` keep cases whose attribute reads as a
/// number within the bound.
fn parse_where(filter: Filter, condition: &str) -> Result<Filter, String> {
    let operator = ["<=", ">=", "="]
        .iter()
        .filter_map(|operator| condition.find(operator).map(|at| (at, *operator)))
        .min();
    let Some((at, operator)) = operator else {
        if condition.contains(['<', '>']) {
            return Err(format!(
                "--where uses >= or <= for numbers (both include the bound), got '{condition}'"
            ));
        }
        return Err(format!(
            "--where expects <attribute>=<value>, <attribute>>=<number> or <attribute><=<number>, got '{condition}'"
        ));
    };
    let name = condition[..at].trim();
    let value = condition[at + operator.len()..].trim();
    if operator == "=" {
        if name.ends_with(['<', '>']) {
            return Err(format!(
                "--where uses >= or <= for numbers (both include the bound), got '{condition}'"
            ));
        }
        return Ok(filter.attribute(name, value));
    }
    let number = pm_core::parse_number(value)
        .ok_or_else(|| format!("--where needs a number after {operator}, got '{value}'"))?;
    Ok(if operator == ">=" {
        filter.number_between(name, Some(number), None)
    } else {
        filter.number_between(name, None, Some(number))
    })
}

/// Parses a view-filter flag shared by analysis and evidence commands.
fn parse_filter_flag(
    flag: &str,
    args: &[String],
    i: &mut usize,
    filter: &mut Filter,
) -> Result<bool, String> {
    *filter = match flag {
        "--with-activity" => {
            let value = take(args, i, "--with-activity")?;
            std::mem::take(filter).with_activity(value.trim())
        }
        "--without-activity" => {
            let value = take(args, i, "--without-activity")?;
            std::mem::take(filter).without_activity(value.trim())
        }
        "--starts-with" => {
            let value = take(args, i, "--starts-with")?;
            std::mem::take(filter).starts_with(value.trim())
        }
        "--ends-with" => {
            let value = take(args, i, "--ends-with")?;
            std::mem::take(filter).ends_with(value.trim())
        }
        "--directly-follows" => {
            let from = take(args, i, "--directly-follows")?;
            let to = take(args, i, "--directly-follows")?;
            std::mem::take(filter).directly_follows(from.trim(), to.trim())
        }
        "--eventually-follows" => {
            let from = take(args, i, "--eventually-follows")?;
            let to = take(args, i, "--eventually-follows")?;
            std::mem::take(filter).eventually_follows(from.trim(), to.trim())
        }
        "--where" => {
            let value = take(args, i, "--where")?;
            parse_where(std::mem::take(filter), &value)?
        }
        "--top-variants" => {
            let value = take(args, i, "--top-variants")?;
            let count = value
                .parse()
                .map_err(|_| format!("--top-variants expects a number, got '{value}'"))?;
            std::mem::take(filter).top_variants(count)
        }
        "--min-case-events" => {
            let value = take(args, i, "--min-case-events")?;
            let count = value
                .parse()
                .map_err(|_| format!("--min-case-events expects a number, got '{value}'"))?;
            std::mem::take(filter).min_events(count)
        }
        "--slower-than" => {
            let value = take(args, i, "--slower-than")?;
            std::mem::take(filter).min_duration(parse_duration(&value)?)
        }
        "--faster-than" => {
            let value = take(args, i, "--faster-than")?;
            std::mem::take(filter).max_duration(parse_duration(&value)?)
        }
        "--complete-only" => std::mem::take(filter).complete_only(true),
        "--latest" => {
            let value = take(args, i, "--latest")?;
            let (name, wanted) = value
                .split_once('=')
                .ok_or_else(|| format!("--latest expects <attribute>=<value>, got '{value}'"))?;
            let mut next = std::mem::take(filter);
            next.attribute_latest
                .entry(name.trim().to_owned())
                .or_default()
                .insert(wanted.trim().to_owned());
            next
        }
        "--repeats" => {
            let value = take(args, i, "--repeats")?;
            let mut next = std::mem::take(filter);
            if value.trim().eq_ignore_ascii_case("any") {
                next.repeats_any = true;
            } else {
                next.repeats.insert(value.trim().to_owned());
            }
            next
        }
        "--handled-by" => {
            let value = take(args, i, "--handled-by")?;
            let mut next = std::mem::take(filter);
            next.handled_by.insert(value.trim().to_owned());
            next
        }
        _ => return Ok(false),
    };
    Ok(true)
}

/// Resolves the mapping from a project and any explicit column flags.
fn resolve_mapping(
    prep: &Prep,
    case: Option<String>,
    activity: Option<String>,
    timestamp: Option<String>,
) -> Result<Option<Mapping>, String> {
    match (case, activity, timestamp) {
        (Some(c), Some(a), Some(t)) => {
            let mut m = Mapping::new(&c, &a, &t);
            if let Some(saved) = &prep.mapping {
                m.event_order = saved.event_order.clone();
                m.end_timestamp = saved.end_timestamp.clone();
                m.resource = saved.resource.clone();
                m.lifecycle = saved.lifecycle.clone();
                m.cost = saved.cost.clone();
                m.case_attributes = saved.case_attributes.clone();
            }
            Ok(Some(m))
        }
        (None, None, None) => Ok(prep.mapping.clone()),
        _ => Err(
            "specify all three of --case, --activity-column and --timestamp, or none of them"
                .to_owned(),
        ),
    }
}

/// Loads the normalized log under the given settings, reporting a blocking
/// decision the same way `analyze` does rather than failing obscurely.
fn load_or_report(
    path: &str,
    prep: &Prep,
    mapping: Option<Mapping>,
) -> Result<Option<pm_query::LoadedLog>, String> {
    match pm_query::load_log(path, mapping, &prep.policy, prep.rules.clone())
        .map_err(|e| e.to_string())?
    {
        Ok(log) => Ok(Some(log)),
        Err(blocked) => {
            print_blocked(&blocked.report, false);
            Ok(None)
        }
    }
}

fn case_command(args: &[String]) -> Result<ExitCode, String> {
    let mut positional: Vec<String> = Vec::new();
    let mut prep = Prep::default();
    let (mut case, mut activity, mut timestamp) = (None, None, None);
    let mut filter = Filter::new();
    let mut filter_explicit = false;
    let mut json = false;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--json" {
            json = true;
        } else if parse_prep_flag(
            arg,
            args,
            &mut i,
            &mut prep,
            &mut case,
            &mut activity,
            &mut timestamp,
        )? {
            // handled
        } else if parse_filter_flag(arg, args, &mut i, &mut filter)? {
            filter_explicit = true;
        } else if arg.starts_with('-') {
            return Err(format!("unknown option '{arg}'"));
        } else {
            positional.push(arg.to_owned());
        }
        i += 1;
    }

    let path = positional
        .first()
        .ok_or("case needs a file and a case id, e.g. `glasstarn case orders.csv ORDER-1042`")?;
    let wanted = positional
        .get(1)
        .ok_or("case needs a case id, e.g. `glasstarn case orders.csv ORDER-1042`")?;

    let mapping = resolve_mapping(&prep, case, activity, timestamp)?;
    let filter = resolve_filter(&prep, filter_explicit, filter);
    let Some(loaded) = load_or_report(path, &prep, mapping)? else {
        return Ok(ExitCode::from(3));
    };
    let filtered = filter.apply(&loaded.log);

    let Some(detail) =
        pm_query::inspect_case_read_as(&filtered.log, wanted, loaded.timestamp_reading)
    else {
        return Err(format!(
            "no case '{wanted}' in the current view of {path}. Case ids are matched exactly."
        ));
    };

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&detail).map_err(|e| e.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }

    println!("{}", detail.case);
    println!(
        "{} · {} total{}",
        plural(detail.events.len(), "event", "events"),
        duration(detail.throughput_seconds),
        if !detail.throughput_eligible {
            " · INCOMPLETE (excluded from duration averages)"
        } else {
            ""
        }
    );
    if !detail.attributes.is_empty() {
        let pairs: Vec<String> = detail
            .attributes
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect();
        println!("{}", pairs.join(" · "));
    }
    if let Some(cost) = detail.total_cost_microunits {
        println!("total cost {}", cost_units(cost));
    }
    println!("timestamps read {}", detail.timestamp_reading);

    // The column header states which measurement it is, rather than "GAP",
    // which would read as waiting whether or not it is.
    let gap_header = if detail.gap_is_waiting {
        "WAITING"
    } else {
        "ELAPSED"
    };
    println!();
    println!(
        "{:>5}  {:<19}  {gap_header:>9}  {:<22}  WHO",
        "ROW", "WHEN", "ACTIVITY"
    );
    for event in &detail.events {
        println!(
            "{:>5}  {:<19}  {:>9}  {:<22}  {}",
            event.source_row,
            event.timestamp,
            event
                .since_previous_seconds
                .map(duration)
                .unwrap_or_else(|| "—".to_owned()),
            truncate(&event.activity, 22),
            event.resource.clone().unwrap_or_default()
        );
    }
    if !detail.gap_is_waiting {
        println!(
            "\nELAPSED is start-to-start: it includes each activity's own duration,\n\
             because not every event has an end timestamp."
        );
    }
    println!("Row numbers match the source file, so each event can be checked there.");
    Ok(ExitCode::SUCCESS)
}

/// Lists every account so a person or an assistant can say what it is.
fn resources_command(args: &[String]) -> Result<ExitCode, String> {
    let (path, prep, mapping, json) = parse_targeting_args(args, "resources")?;
    let report = match pm_query::resource_report(&path, mapping, &prep.policy, prep.rules.clone())
        .map_err(|e| e.to_string())?
    {
        Ok(r) => r,
        Err(blocked) => {
            print_blocked(&blocked.report, false);
            return Ok(ExitCode::from(3));
        }
    };

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }

    if !report.resources_mapped {
        println!(
            "No resource column is mapped, so who did the work is unknown.\n\
             Map one with --resource <column> to see where the machine stops and people start."
        );
        return Ok(ExitCode::from(3));
    }

    println!(
        "{}\n",
        plural(report.resources.len(), "account", "accounts")
    );
    println!(
        "{:<24} {:>8} {:>7} {:>7} {:>9}  LOOKS LIKE",
        "ACCOUNT", "EVENTS", "CASES", "STARTS", "DECIDED"
    );
    for r in &report.resources {
        println!(
            "{:<24} {:>8} {:>7} {:>7} {:>9}  {}",
            truncate(&r.resource, 24),
            r.events,
            r.cases,
            r.starts_cases,
            match r.decided {
                Some(ResourceKind::System) => "system",
                Some(ResourceKind::Human) => "person",
                Some(ResourceKind::Unrecorded) | None => "—",
            },
            r.suggested_because
        );
    }

    if report.undecided > 0 {
        println!(
            "\n{} not yet decided. Until they are, every account counts as a person,\n\
             which overstates manual work rather than hiding it.",
            plural(report.undecided, "account is", "accounts are")
        );
        println!(
            "\nRecord decisions in a rules file:\n\
             \x20 {{ \"resource_kinds\": {{ \"WF-BATCH\": \"system\", \"alice\": \"human\" }} }}\n\
             then run with --rules <file>. An assistant can propose one from this table."
        );
    } else {
        println!("\nEvery account is decided. `glasstarn automation` will use these.");
    }
    Ok(ExitCode::SUCCESS)
}

/// Reports where the machine stops and people start.
fn automation_command(args: &[String]) -> Result<ExitCode, String> {
    let (path, prep, mapping, json) = parse_targeting_args(args, "automation")?;
    let report = match pm_query::automation_report(&path, mapping, &prep.policy, prep.rules.clone())
        .map_err(|e| e.to_string())?
    {
        Ok(r) => r,
        Err(blocked) => {
            print_blocked(&blocked.report, false);
            return Ok(ExitCode::from(3));
        }
    };

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }

    println!("{}", report.source);
    if report.resources_mapped && !report.unclassified_resources.is_empty() {
        println!(
            "NOT YET CLASSIFIED  {} not decided as person or system ({} steps).\n\
             \x20                   The straight-through rate appears once they are decided.\n\
             \x20                   Run `glasstarn resources {}` to see them.",
            plural(
                report.unclassified_resources.len(),
                "account is",
                "accounts are"
            ),
            report.unclassified_events,
            report.source
        );
        if !report.suggested_system_resources.is_empty() {
            println!(
                "LOOK LIKE SYSTEMS   {} (please confirm)",
                report.suggested_system_resources.join(", ")
            );
        }
    } else {
        match report.straight_through_percent {
            Some(percent) => println!(
                "{:.0}% of classifiable cases ran straight through — {} untouched, {} needed a person",
                percent, report.straight_through_cases, report.touched_cases
            ),
            None => println!(
                "Straight-through rate is unknown: no resource column is mapped, so who did\n\
                 the work was never recorded. Map one with --resource <column>."
            ),
        }
        if report.undetermined_cases > 0 {
            println!(
                "{:.0}% performer coverage · {} could not be classified because performer data was missing",
                report.classification_coverage_percent,
                plural(report.undetermined_cases, "case", "cases")
            );
        }
    }

    if !report.manual_steps_ranked.is_empty() {
        println!("\nMANUAL-WORK HOTSPOTS — INVESTIGATION ORDER");
        println!(
            "  Ranked by {} × routing-context concentration. This measures where a step\n\
             \x20 appears in the process, not whether its inputs or decisions are rules-based.",
            match report.ranking_basis {
                pm_core::RankingBasis::ProcessingTime => "processing time",
                pm_core::RankingBasis::Occurrences => "occurrences",
            }
        );
        println!(
            "\n{:<24} {:>9} {:>9} {:>7} {:>13}  EXAMPLES",
            "ACTIVITY", "PROCESS", "WAIT", "CASES", "ROUTING CONC."
        );
        for step in report.manual_steps_ranked.iter().take(10) {
            println!(
                "{:<24} {:>9} {:>9} {:>7} {:>12.0}%  {}",
                truncate(&step.activity, 24),
                duration(step.processing_seconds),
                duration(step.waiting_before_seconds),
                step.cases,
                step.routing_context_concentration,
                step.example_cases.join(", ")
            );
        }
        println!(
            "\n{} observed manual processing time.",
            duration(report.manual_processing_seconds)
        );
        if report.waiting_before_manual_seconds > 0 {
            println!(
                "{} true waiting immediately before manual steps (reported separately).",
                duration(report.waiting_before_manual_seconds)
            );
        }
        if report.start_to_start_before_manual_seconds > 0 {
            println!(
                "{} start-to-start elapsed before manual steps; this includes prior processing.",
                duration(report.start_to_start_before_manual_seconds)
            );
        }
    }

    if !report.divergences.is_empty() {
        println!(
            "\nWHERE CASES LEAVE THE MOST FREQUENT OBSERVED VARIANT ({} cases, {:.1}%)",
            report.dominant_variant_cases, report.dominant_variant_percent
        );
        for d in report.divergences.iter().take(5) {
            println!(
                "  {:>10}  after {}, did {}; dominant variant next had {}   e.g. {}",
                plural(d.cases as usize, "case", "cases"),
                d.after.as_deref().unwrap_or("the start"),
                d.took.as_deref().unwrap_or("nothing"),
                d.dominant_path_next.as_deref().unwrap_or("nothing"),
                d.example_cases.join(", ")
            );
        }
    }

    println!("\nINTERPRETATION LIMITS");
    for limitation in &report.interpretation_limits {
        println!("  • {limitation}");
    }
    Ok(ExitCode::SUCCESS)
}

fn compare_cohorts_command(args: &[String]) -> Result<ExitCode, String> {
    let mut path = None;
    let mut prep = Prep::default();
    let (mut case, mut activity, mut timestamp) = (None, None, None);
    let mut filter = Filter::new();
    let mut filter_explicit = false;
    let mut attribute = None;
    let mut left = None;
    let mut right = None;
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--json" => json = true,
            "--attribute" => attribute = Some(take(args, &mut i, "--attribute")?),
            "--left" => left = Some(take(args, &mut i, "--left")?),
            "--right" => right = Some(take(args, &mut i, "--right")?),
            other => {
                if parse_prep_flag(
                    other,
                    args,
                    &mut i,
                    &mut prep,
                    &mut case,
                    &mut activity,
                    &mut timestamp,
                )? {
                    // handled
                } else if parse_filter_flag(other, args, &mut i, &mut filter)? {
                    filter_explicit = true;
                } else if other.starts_with('-') {
                    return Err(format!("unknown option '{other}'"));
                } else if path.replace(other.to_owned()).is_some() {
                    return Err("compare-cohorts accepts only one source file".to_owned());
                }
            }
        }
        i += 1;
    }
    let path = path
        .or_else(|| prep.project_source.clone())
        .ok_or("compare-cohorts needs a file or --project")?;
    let attribute = attribute.ok_or("compare-cohorts requires --attribute <name>")?;
    let left = left.ok_or("compare-cohorts requires --left <value>")?;
    let right = right.ok_or("compare-cohorts requires --right <value>")?;
    let mapping = resolve_mapping(&prep, case, activity, timestamp)?;
    let filter = resolve_filter(&prep, filter_explicit, filter);
    let Some(loaded) = load_or_report(&path, &prep, mapping)? else {
        return Ok(ExitCode::from(3));
    };
    let filtered = filter.apply(&loaded.log);
    let comparison = pm_query::compare_cohorts(&filtered.log, &attribute, &left, &right)
        .map_err(|error| error.to_string())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&comparison).map_err(|error| error.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }
    println!("{}: {} vs {}", comparison.attribute, left, right);
    println!(
        "{} cases vs {} cases · mean throughput change {}",
        comparison.left.cases,
        comparison.right.cases,
        signed_duration(comparison.mean_throughput_difference_seconds)
    );
    println!("\nLARGEST ACTIVITY-PREVALENCE DIFFERENCES");
    for difference in comparison.activity_differences.iter().take(10) {
        println!(
            "  {:+6.1} points  {:<28} {:>5.1}% → {:>5.1}%",
            difference.difference_points,
            truncate(&difference.activity, 28),
            difference.left_case_percent,
            difference.right_case_percent
        );
    }
    println!("\nINTERPRETATION LIMITS");
    for limitation in &comparison.interpretation_limits {
        println!("  • {limitation}");
    }
    Ok(ExitCode::SUCCESS)
}

fn signed_duration(seconds: i64) -> String {
    if seconds > 0 {
        format!("+{}", duration(seconds))
    } else if seconds < 0 {
        format!("-{}", duration(seconds.saturating_abs()))
    } else {
        duration(0)
    }
}

fn values_command(args: &[String]) -> Result<ExitCode, String> {
    let mut path = None;
    let mut prep = Prep::default();
    let (mut case, mut activity, mut timestamp) = (None, None, None);
    let mut filter = Filter::new();
    let mut filter_explicit = false;
    let mut field = None;
    let mut grouping = pm_query::ValueGrouping::None;
    let mut limit = 10usize;
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--json" => json = true,
            "--field" => field = Some(take(args, &mut i, "--field")?),
            "--by" => {
                grouping = match take(args, &mut i, "--by")?.as_str() {
                    "variant" | "route" => pm_query::ValueGrouping::Variant,
                    "last-step" | "step" => pm_query::ValueGrouping::LastStep,
                    other => pm_query::ValueGrouping::Attribute(other.to_owned()),
                }
            }
            "--limit" => {
                let value = take(args, &mut i, "--limit")?;
                limit = value
                    .parse()
                    .ok()
                    .filter(|limit| *limit > 0)
                    .ok_or_else(|| format!("--limit expects a positive number, got '{value}'"))?;
            }
            other => {
                if parse_prep_flag(
                    other,
                    args,
                    &mut i,
                    &mut prep,
                    &mut case,
                    &mut activity,
                    &mut timestamp,
                )? {
                    // handled
                } else if parse_filter_flag(other, args, &mut i, &mut filter)? {
                    filter_explicit = true;
                } else if other.starts_with('-') {
                    return Err(format!("unknown option '{other}'"));
                } else if path.replace(other.to_owned()).is_some() {
                    return Err("values accepts only one source file".to_owned());
                }
            }
        }
        i += 1;
    }
    let path = path
        .or_else(|| prep.project_source.clone())
        .ok_or("values needs a file or --project")?;
    let field = field.ok_or("values requires --field <name>")?;
    let mapping = resolve_mapping(&prep, case, activity, timestamp)?;
    let filter = resolve_filter(&prep, filter_explicit, filter);
    let finishing = prep
        .rules
        .observation_window
        .as_ref()
        .map(|window| window.end_activities.clone())
        .unwrap_or_default();
    let Some(loaded) = load_or_report(&path, &prep, mapping)? else {
        return Ok(ExitCode::from(3));
    };
    let filtered = filter.apply(&loaded.log);
    let summary = pm_query::summarize_values(&filtered.log, &field, &grouping, &finishing, limit)
        .map_err(|error| error.to_string())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).map_err(|error| error.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }
    let stats = &summary.overall;
    println!(
        "{field}: total {} over {} of {} cases",
        amount(stats.total),
        stats.cases_with_value,
        stats.cases
    );
    if let (Some(mean), Some(median), Some(p90)) = (stats.mean, stats.median, stats.p90) {
        println!(
            "average {} · median {} · 90th percentile {}",
            amount(mean),
            amount(median),
            amount(p90)
        );
    }
    if let (Some(open), Some(total)) = (stats.open_cases, stats.open_total) {
        println!("still open: {} in {open} cases", amount(total));
    }
    if filtered.narrowed() {
        println!(
            "view: {:.1}% of cases ({} of {})",
            filtered.cases_kept_percent(),
            filtered.log.case_count(),
            filtered.cases_before
        );
    }
    if !summary.groups.is_empty() {
        println!("\nBY GROUP (largest total first)");
        for group in &summary.groups {
            println!(
                "  {:>14}  {:>6} cases  {}",
                amount(group.stats.total),
                group.stats.cases,
                truncate(&group.label, 60)
            );
        }
        if let Some(other) = &summary.other_groups {
            println!(
                "  {:>14}  {:>6} cases  ({} more groups)",
                amount(other.total),
                other.cases,
                summary.groups_available - summary.groups.len()
            );
        }
    }
    for note in &summary.semantics {
        println!("  • {note}");
    }
    Ok(ExitCode::SUCCESS)
}

/// An amount with thousands separators and at most two decimals.
fn amount(value: f64) -> String {
    let rounded = format!("{:.2}", value.abs());
    let (whole, fraction) = rounded.split_once('.').unwrap_or((&rounded, "00"));
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let sign = if value < 0.0 && rounded != "0.00" {
        "-"
    } else {
        ""
    };
    if fraction == "00" {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}.{fraction}")
    }
}

fn compare_periods_command(args: &[String]) -> Result<ExitCode, String> {
    let mut path = None;
    let mut prep = Prep::default();
    let (mut case, mut activity, mut timestamp) = (None, None, None);
    let mut filter = Filter::new();
    let mut filter_explicit = false;
    let mut baseline_start = None;
    let mut baseline_end = None;
    let mut comparison_start = None;
    let mut comparison_end = None;
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--json" => json = true,
            "--baseline-start" => {
                baseline_start = Some(take_i64(args, &mut i, "--baseline-start")?)
            }
            "--baseline-end" => baseline_end = Some(take_i64(args, &mut i, "--baseline-end")?),
            "--comparison-start" => {
                comparison_start = Some(take_i64(args, &mut i, "--comparison-start")?)
            }
            "--comparison-end" => {
                comparison_end = Some(take_i64(args, &mut i, "--comparison-end")?)
            }
            other => {
                if parse_prep_flag(
                    other,
                    args,
                    &mut i,
                    &mut prep,
                    &mut case,
                    &mut activity,
                    &mut timestamp,
                )? {
                    // handled
                } else if parse_filter_flag(other, args, &mut i, &mut filter)? {
                    filter_explicit = true;
                } else if other.starts_with('-') {
                    return Err(format!("unknown option '{other}'"));
                } else if path.replace(other.to_owned()).is_some() {
                    return Err("compare-periods accepts only one source file".to_owned());
                }
            }
        }
        i += 1;
    }
    let path = path
        .or_else(|| prep.project_source.clone())
        .ok_or("compare-periods needs a file or --project")?;
    let baseline_start = baseline_start.ok_or("compare-periods requires --baseline-start")?;
    let baseline_end = baseline_end.ok_or("compare-periods requires --baseline-end")?;
    let comparison_start = comparison_start.ok_or("compare-periods requires --comparison-start")?;
    let comparison_end = comparison_end.ok_or("compare-periods requires --comparison-end")?;
    let mapping = resolve_mapping(&prep, case, activity, timestamp)?;
    let filter = resolve_filter(&prep, filter_explicit, filter);
    let Some(loaded) = load_or_report(&path, &prep, mapping)? else {
        return Ok(ExitCode::from(3));
    };
    let filtered = filter.apply(&loaded.log);
    let comparison = pm_query::compare_periods(
        &filtered.log,
        baseline_start,
        baseline_end,
        comparison_start,
        comparison_end,
    )
    .map_err(|error| error.to_string())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&comparison).map_err(|error| error.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "{} baseline cases vs {} comparison cases · mean throughput change {}",
        comparison.baseline.cases,
        comparison.comparison.cases,
        signed_duration(comparison.mean_throughput_difference_seconds)
    );
    println!("\nLARGEST ACTIVITY-PREVALENCE CHANGES");
    for difference in comparison.activity_differences.iter().take(10) {
        println!(
            "  {:+6.1} points  {:<28} {:>5.1}% → {:>5.1}%",
            difference.difference_points,
            truncate(&difference.activity, 28),
            difference.left_case_percent,
            difference.right_case_percent
        );
    }
    println!("\nINTERPRETATION LIMITS");
    for limitation in &comparison.interpretation_limits {
        println!("  • {limitation}");
    }
    Ok(ExitCode::SUCCESS)
}

fn take_i64(args: &[String], index: &mut usize, flag: &str) -> Result<i64, String> {
    let value = take(args, index, flag)?;
    value
        .parse()
        .map_err(|_| format!("{flag} expects integer Unix microseconds, got '{value}'"))
}

/// Shared argument parsing for the targeting commands.
fn parse_targeting_args(
    args: &[String],
    command: &str,
) -> Result<(String, Prep, Option<Mapping>, bool), String> {
    let mut positional: Vec<String> = Vec::new();
    let mut prep = Prep::default();
    let (mut case, mut activity, mut timestamp) = (None, None, None);
    let mut json = false;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--json" {
            json = true;
        } else if parse_prep_flag(
            arg,
            args,
            &mut i,
            &mut prep,
            &mut case,
            &mut activity,
            &mut timestamp,
        )? {
            // handled
        } else if arg.starts_with('-') {
            return Err(format!("unknown option '{arg}'"));
        } else {
            positional.push(arg.to_owned());
        }
        i += 1;
    }

    let path = positional
        .first()
        .ok_or(format!("{command} needs a file"))?
        .clone();
    let mapping = resolve_mapping(&prep, case, activity, timestamp)?;
    Ok((path, prep, mapping, json))
}

fn cases_command(args: &[String]) -> Result<ExitCode, String> {
    let mut positional: Vec<String> = Vec::new();
    let mut prep = Prep::default();
    let (mut case, mut activity, mut timestamp) = (None, None, None);
    let mut json = false;
    let mut limit = 20usize;
    let mut filter = Filter::new();
    let mut filter_explicit = false;
    let mut evidence: Option<pm_query::Evidence> = None;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--json" => json = true,
            "--limit" => {
                limit = take(args, &mut i, "--limit")?
                    .parse()
                    .map_err(|_| "--limit expects a number".to_owned())?;
            }
            "--activity" => {
                evidence = Some(pm_query::Evidence::Activity(take(
                    args,
                    &mut i,
                    "--activity",
                )?));
            }
            "--transition" => {
                let from = take(args, &mut i, "--transition")?;
                let to = take(args, &mut i, "--transition")?;
                evidence = Some(pm_query::Evidence::Transition { from, to });
            }
            "--handover" => {
                let from = take(args, &mut i, "--handover")?;
                let to = take(args, &mut i, "--handover")?;
                evidence = Some(pm_query::Evidence::Handover { from, to });
            }
            "--variant" => {
                // The whole path, so a variant row leads to its cases.
                let path = take(args, &mut i, "--variant")?;
                let steps: Vec<String> = path
                    .split(&['>', ','][..])
                    .map(|s| s.trim().trim_start_matches('-').trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect();
                if steps.len() < 2 {
                    return Err(
                        "--variant expects a path, e.g. \"Receive > Check > Ship\"".to_owned()
                    );
                }
                evidence = Some(pm_query::Evidence::Variant(steps));
            }
            other => {
                if parse_prep_flag(
                    other,
                    args,
                    &mut i,
                    &mut prep,
                    &mut case,
                    &mut activity,
                    &mut timestamp,
                )? {
                    // handled
                } else if parse_filter_flag(other, args, &mut i, &mut filter)? {
                    filter_explicit = true;
                } else if other.starts_with('-') {
                    return Err(format!("unknown option '{other}'"));
                } else {
                    positional.push(other.to_owned());
                }
            }
        }
        i += 1;
    }

    let path = positional.first().ok_or("cases needs a file")?;
    let evidence = evidence.ok_or(
        "say what to look for: --activity <name>, --transition <from> <to>, \
         --handover <from> <to>, or --variant \"A > B > C\"",
    )?;

    let mapping = resolve_mapping(&prep, case, activity, timestamp)?;
    let filter = resolve_filter(&prep, filter_explicit, filter);
    let Some(loaded) = load_or_report(path, &prep, mapping)? else {
        return Ok(ExitCode::from(3));
    };
    let filtered = filter.apply(&loaded.log);
    let log = &filtered.log;

    let total = pm_query::count_cases(log, &evidence);
    let found = pm_query::find_cases(log, &evidence, limit);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "total": total,
                "shown": found.len(),
                "cases": found,
            }))
            .map_err(|e| e.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }

    if total == 0 {
        println!("No cases match. Names are matched exactly as they appear in the log.");
        return Ok(ExitCode::from(3));
    }

    println!("{} matching", plural(total, "case", "cases"));
    for label in &found {
        println!("  {label}");
    }
    if total > found.len() {
        println!("  … and {} more", total - found.len());
    }
    println!("\nOpen any of them with `glasstarn case {path} <case-id>`.");
    Ok(ExitCode::SUCCESS)
}

fn inspect_command(args: &[String]) -> Result<ExitCode, String> {
    let mut path = None;
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            other if other.starts_with('-') => {
                return Err(format!("unknown inspect option '{other}'"));
            }
            other if path.replace(other.to_owned()).is_some() => {
                return Err("inspect accepts only one source file".to_owned());
            }
            _ => {}
        }
    }
    let path = path.ok_or("inspect needs a file. Try `glasstarn inspect orders.csv`.")?;
    let profile = pm_query::inspect_log(&path, true).map_err(|e| e.to_string())?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&profile).map_err(|error| error.to_string())?
        );
        return Ok(if profile.unfillable_roles.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(3)
        });
    }

    println!("{path}");
    println!(
        "{} · {} sampled\n",
        plural(profile.columns.len(), "column", "columns"),
        plural(profile.rows_sampled, "row", "rows"),
    );

    let width = profile
        .columns
        .iter()
        .map(|c| c.name.len())
        .max()
        .unwrap_or(6)
        .clamp(6, 28);

    println!(
        "{:<width$}  {:<22}  {:>7}  {:>6}  EXAMPLES",
        "COLUMN", "LOOKS LIKE", "DISTINCT", "BLANK"
    );
    for c in &profile.columns {
        let examples = c
            .examples
            .iter()
            .map(|e| truncate(e, 18))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "{:<width$}  {:<22}  {:>7}  {:>5.0}%  {}",
            truncate(&c.name, width),
            truncate(&c.kind, 22),
            c.distinct,
            c.blank_percent,
            examples
        );
    }

    println!("\nBEST MATCH FOR EACH FIELD");
    for role in &profile.roles {
        let candidates = &role.candidates;
        let marker = if role.required { "*" } else { " " };
        let Some(best) = candidates.first() else {
            println!("{marker} {:<15} — nothing suitable found", role.role);
            continue;
        };
        // Only a couple of alternatives: on a file with forty columns, listing
        // every possibility is no more useful than listing none.
        let alternates: Vec<&str> = candidates
            .iter()
            .skip(1)
            .take(2)
            .map(String::as_str)
            .collect();
        let extra = candidates.len().saturating_sub(1 + alternates.len());
        let mut line = format!("{marker} {:<15} {}", role.role, best);
        if !alternates.is_empty() {
            line.push_str(&format!("   (or {}", alternates.join(", ")));
            if extra > 0 {
                line.push_str(&format!(", +{extra} more"));
            }
            line.push(')');
        }
        println!("{line}");
    }
    println!("  * required");

    if !profile.unfillable_roles.is_empty() {
        println!(
            "\nThis file cannot be analysed as it stands: no column suits {}.",
            profile.unfillable_roles.join(" or ")
        );
        println!(
            "If it has one row per case with several timestamp columns, analysing it will \
             unpivot it automatically."
        );
        return Ok(ExitCode::from(3));
    }

    println!("\nEverything else becomes a case attribute, kept for filtering and comparison.");
    Ok(ExitCode::SUCCESS)
}

/// Shortens a value for display, marking that it was cut.
fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        value.to_owned()
    } else {
        format!(
            "{}…",
            value
                .chars()
                .take(width.saturating_sub(1))
                .collect::<String>()
        )
    }
}

fn import_bpmn_command(args: &[String]) -> Result<ExitCode, String> {
    let mut path = None;
    let mut json = false;
    for arg in args {
        if arg == "--json" {
            json = true;
        } else if arg.starts_with('-') {
            return Err(format!("unknown import-bpmn option '{arg}'"));
        } else if path.replace(arg.clone()).is_some() {
            return Err("import-bpmn accepts only one BPMN file".to_owned());
        }
    }
    let path = path.ok_or("import-bpmn needs a .bpmn file")?;
    let model = pm_query::import_bpmn_model(&path).map_err(|error| error.to_string())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&model).map_err(|error| error.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }
    println!("{}", model.source);
    println!("BPMN 2.0 EXECUTABLE MODEL");
    println!(
        "process: {}{}",
        model.process_id,
        model
            .process_name
            .as_ref()
            .map_or(String::new(), |name| format!(" ({name})"))
    );
    println!(
        "{} places · {} transitions · {} observable activities",
        model.net.place_count,
        model.net.transitions.len(),
        model
            .net
            .transitions
            .iter()
            .filter(|transition| transition.activity.is_some())
            .count()
    );
    println!("Use --json to save this model for `glasstarn conformance --model`.");
    Ok(ExitCode::SUCCESS)
}

fn conformance_command(args: &[String]) -> Result<ExitCode, String> {
    let mut path: Option<String> = None;
    let mut model_path: Option<String> = None;
    let mut prep = Prep::default();
    let (mut case, mut activity, mut timestamp) = (None, None, None);
    let mut filter = Filter::new();
    let mut filter_explicit = false;
    let mut limit = 20usize;
    let mut alignment = false;
    let mut json = false;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--json" => json = true,
            "--activity" => activity = Some(take(args, &mut i, "--activity")?),
            "--model" => model_path = Some(take(args, &mut i, "--model")?),
            "--algorithm" => {
                let value = take(args, &mut i, "--algorithm")?;
                alignment = match value.as_str() {
                    "token" => false,
                    "alignment" => true,
                    _ => {
                        return Err(format!(
                            "--algorithm expects 'token' or 'alignment', got '{value}'"
                        ));
                    }
                };
            }
            "--limit" => {
                let value = take(args, &mut i, "--limit")?;
                limit = value
                    .parse()
                    .map_err(|_| format!("--limit expects a number, got '{value}'"))?;
            }
            other => {
                if parse_prep_flag(
                    other,
                    args,
                    &mut i,
                    &mut prep,
                    &mut case,
                    &mut activity,
                    &mut timestamp,
                )? {
                    // handled
                } else if parse_filter_flag(other, args, &mut i, &mut filter)? {
                    filter_explicit = true;
                } else if other.starts_with('-') {
                    return Err(format!("unknown conformance option '{other}'"));
                } else if path.replace(other.to_owned()).is_some() {
                    return Err("conformance accepts only one event-log file".to_owned());
                }
            }
        }
        i += 1;
    }

    let path = path
        .or_else(|| prep.project_source.clone())
        .ok_or("conformance needs an event-log file or --project")?;
    let model_path = model_path.ok_or(
        "conformance needs --model <model.json>; create one with `glasstarn discover baseline.csv --json`",
    )?;
    let model_json = std::fs::read(&model_path)
        .map_err(|error| format!("could not read model '{model_path}': {error}"))?;
    let model = pm_query::parse_formal_model_json(&model_json)
        .map_err(|error| format!("'{model_path}' {error}"))?;
    let mapping = resolve_mapping(&prep, case, activity, timestamp)?;
    let filter = resolve_filter(&prep, filter_explicit, filter);
    let loaded = match pm_query::load_log(&path, mapping, &prep.policy, prep.rules.clone())
        .map_err(|error| error.to_string())?
    {
        Ok(loaded) => loaded,
        Err(blocked) => {
            print_blocked(&blocked.report, json);
            return Ok(ExitCode::from(3));
        }
    };
    if alignment {
        let report = pm_query::align_conformance(
            &path,
            &model.source,
            &loaded.log,
            &filter,
            &model.net,
            limit,
        )
        .map_err(|error| error.to_string())?;
        return print_alignment_report(&report, json);
    }
    let report = pm_query::check_conformance(
        &path,
        &model.source,
        &loaded.log,
        &filter,
        &model.net,
        limit,
    )
    .map_err(|error| error.to_string())?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }

    println!("{}", report.source);
    if report.view.narrowed {
        println!(
            "FILTERED VIEW — showing {} of {} cases ({:.0}%)",
            report.view.cases_shown, report.view.cases_total, report.view.percent_shown
        );
    }
    println!("CONFORMANCE — TOKEN-BASED REPLAY");
    println!("model: {}", report.model_source);
    println!(
        "{:.2}% token fitness · {} of {} cases fit ({:.1}%)",
        report.fitness_percent, report.fit_cases, report.view.cases_shown, report.fit_case_percent
    );
    println!(
        "{} missing · {} remaining · {} not in model",
        plural(report.missing_tokens as usize, "token", "tokens"),
        plural(report.remaining_tokens as usize, "token", "tokens"),
        plural(report.unmapped_events as usize, "event", "events")
    );
    println!(
        "Token fitness is an approximation; silent and parallel structures can make it optimistic."
    );

    if report.deviating_cases == 0 {
        println!("\nEvery case in this view replayed without a token deviation.");
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "\nWORST DEVIATING CASES — showing {} of {}",
        report.deviations_shown, report.deviating_cases
    );
    for case in &report.cases {
        println!(
            "\n{} — {:.2}% fitness · {} missing · {} remaining · {} outside model",
            case.case,
            case.fitness_millionths as f64 / 10_000.0,
            case.missing_tokens,
            case.remaining_tokens,
            case.unmapped_events
        );
        for deviation in &case.deviations {
            match deviation {
                pm_query::DeviationEvidence::ActivityNotInModel {
                    event_index,
                    source_row,
                    activity,
                } => println!(
                    "  row {source_row}, event {}: {activity} is not in the model",
                    event_index + 1
                ),
                pm_query::DeviationEvidence::MissingTokens {
                    event_index,
                    source_row,
                    activity,
                    count,
                } => println!(
                    "  row {source_row}, event {}: {activity} required {}",
                    event_index + 1,
                    plural(*count as usize, "missing token", "missing tokens")
                ),
                pm_query::DeviationEvidence::FinalTokensMissing { count } => println!(
                    "  case end: {} needed to reach the final marking",
                    plural(*count as usize, "token was", "tokens were")
                ),
                pm_query::DeviationEvidence::TokensRemaining { count } => println!(
                    "  case end: {} remained outside the final marking",
                    plural(*count as usize, "token", "tokens")
                ),
            }
        }
    }
    if report.deviations_shown < report.deviating_cases {
        println!(
            "\n{} more deviating cases omitted; use --limit to change the evidence bound.",
            report.deviating_cases - report.deviations_shown
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn print_alignment_report(
    report: &pm_query::AlignmentReport,
    json: bool,
) -> Result<ExitCode, String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(report).map_err(|error| error.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }
    println!("{}", report.source);
    if report.view.narrowed {
        println!(
            "FILTERED VIEW — showing {} of {} cases ({:.0}%)",
            report.view.cases_shown, report.view.cases_total, report.view.percent_shown
        );
    }
    println!("CONFORMANCE — OPTIMAL A* ALIGNMENT");
    println!("model: {}", report.model_source);
    println!(
        "{:.2}% alignment fitness · {} of {} cases fit ({:.1}%) · total cost {}",
        report.fitness_percent,
        report.fit_cases,
        report.view.cases_shown,
        report.fit_case_percent,
        report.total_cost
    );
    if report.deviating_cases == 0 {
        println!("\nEvery case in this view has a zero-cost alignment.");
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "\nWORST DEVIATING CASES — showing {} of {}",
        report.deviations_shown, report.deviating_cases
    );
    for case in &report.cases {
        println!(
            "\n{} — cost {} · {:.2}% fitness · {} synchronous moves",
            case.case,
            case.cost,
            case.fitness_millionths as f64 / 10_000.0,
            case.synchronous_moves
        );
        for deviation in &case.deviations {
            match deviation {
                pm_query::AlignmentDeviation::LogOnly {
                    event_index,
                    source_row,
                    activity,
                } => println!(
                    "  row {source_row}, event {}: unexpected {activity} (log-only move)",
                    event_index + 1
                ),
                pm_query::AlignmentDeviation::VisibleModel {
                    transition,
                    activity,
                } => println!("  model expected {activity} (model-only transition {transition})"),
            }
        }
    }
    if report.deviations_shown < report.deviating_cases {
        println!(
            "\n{} more deviating cases omitted; use --limit to change the evidence bound.",
            report.deviating_cases - report.deviations_shown
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn discover_command(args: &[String]) -> Result<ExitCode, String> {
    let mut path: Option<String> = None;
    let mut prep = Prep::default();
    let (mut case, mut activity, mut timestamp) = (None, None, None);
    let mut filter = Filter::new();
    let mut filter_explicit = false;
    let mut json = false;
    let mut config = pm_core::InductiveMinerConfig::default();
    let mut alpha = false;
    let mut heuristics = false;
    let mut noise_explicit = false;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--json" => json = true,
            "--activity" => activity = Some(take(args, &mut i, "--activity")?),
            "--algorithm" => {
                let value = take(args, &mut i, "--algorithm")?;
                alpha = match value.as_str() {
                    "imf" => {
                        heuristics = false;
                        false
                    }
                    "alpha" => {
                        heuristics = false;
                        true
                    }
                    "heuristics" => {
                        heuristics = true;
                        false
                    }
                    _ => {
                        return Err(format!(
                            "--algorithm expects 'imf', 'alpha', or 'heuristics', got '{value}'"
                        ));
                    }
                };
            }
            "--noise-threshold" => {
                let value = take(args, &mut i, "--noise-threshold")?;
                config.noise_threshold_milli = parse_noise_threshold(&value)?;
                noise_explicit = true;
            }
            other => {
                if parse_prep_flag(
                    other,
                    args,
                    &mut i,
                    &mut prep,
                    &mut case,
                    &mut activity,
                    &mut timestamp,
                )? {
                    // handled
                } else if parse_filter_flag(other, args, &mut i, &mut filter)? {
                    filter_explicit = true;
                } else if other.starts_with('-') {
                    return Err(format!("unknown discover option '{other}'"));
                } else if path.replace(other.to_owned()).is_some() {
                    return Err("discover accepts only one source file".to_owned());
                }
            }
        }
        i += 1;
    }

    let path = path
        .or_else(|| prep.project_source.clone())
        .ok_or("discover needs a file or --project")?;
    let mapping = resolve_mapping(&prep, case, activity, timestamp)?;
    let filter = resolve_filter(&prep, filter_explicit, filter);
    let loaded = match pm_query::load_log(&path, mapping, &prep.policy, prep.rules.clone())
        .map_err(|error| error.to_string())?
    {
        Ok(loaded) => loaded,
        Err(blocked) => {
            print_blocked(&blocked.report, json);
            return Ok(ExitCode::from(3));
        }
    };
    if (alpha || heuristics) && noise_explicit {
        return Err("--noise-threshold applies only to --algorithm imf".to_owned());
    }
    if alpha {
        let discovered = pm_query::discover_alpha_formal(&path, &loaded.log, &filter)
            .map_err(|error| error.to_string())?;
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&discovered).map_err(|error| error.to_string())?
            );
            return Ok(ExitCode::SUCCESS);
        }
        println!("{}", discovered.source);
        if discovered.view.narrowed {
            println!(
                "FILTERED VIEW — showing {} of {} cases ({:.0}%)",
                discovered.view.cases_shown,
                discovered.view.cases_total,
                discovered.view.percent_shown
            );
        }
        println!("FORMAL MODEL — CLASSIC ALPHA MINER (TEACHING ONLY)");
        println!(
            "{} places · {} transitions",
            discovered.net.place_count,
            discovered.net.transitions.len()
        );
        println!(
            "warning: Alpha Miner mishandles noise and short loops; use IMf for production discovery"
        );
        println!("Use --json for markings, transitions, and arcs.");
        return Ok(ExitCode::SUCCESS);
    }
    if heuristics {
        let discovered = pm_query::discover_heuristics_formal(
            &path,
            &loaded.log,
            &filter,
            pm_core::HeuristicsMinerConfig::default(),
        )
        .map_err(|error| error.to_string())?;
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&discovered).map_err(|error| error.to_string())?
            );
            return Ok(ExitCode::SUCCESS);
        }
        println!("{}", discovered.source);
        if discovered.view.narrowed {
            println!(
                "FILTERED VIEW — showing {} of {} cases ({:.0}%)",
                discovered.view.cases_shown,
                discovered.view.cases_total,
                discovered.view.percent_shown
            );
        }
        println!("HEURISTIC NET — HEURISTICS MINER");
        println!(
            "{} dependency arcs · {} AND-splits · {} AND-joins",
            discovered.arcs.len(),
            discovered.and_splits.len(),
            discovered.and_joins.len()
        );
        for arc in &discovered.arcs {
            println!(
                "  {} → {}  {} observations · {:.1}% dependency · {:?}",
                arc.from,
                arc.to,
                arc.observations,
                arc.dependency_millionths as f64 / 10_000.0,
                arc.kind
            );
        }
        println!(
            "note: heuristic nets may be unsound and are not accepted as conformance baselines"
        );
        println!("Use --json for thresholds, boundaries, and binding scores.");
        return Ok(ExitCode::SUCCESS);
    }
    let discovered = pm_query::discover_formal(&path, &loaded.log, &filter, config)
        .map_err(|error| error.to_string())?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&discovered).map_err(|error| error.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }

    println!("{}", discovered.source);
    if discovered.view.narrowed {
        println!(
            "FILTERED VIEW — showing {} of {} cases ({:.0}%)",
            discovered.view.cases_shown, discovered.view.cases_total, discovered.view.percent_shown
        );
    }
    println!("FORMAL MODEL — INDUCTIVE MINER (IMf)");
    println!(
        "{} · {} · noise threshold {}%",
        plural(discovered.view.cases_shown, "case", "cases"),
        plural(
            discovered
                .net
                .transitions
                .iter()
                .filter(|transition| transition.activity.is_some())
                .count(),
            "observable transition",
            "observable transitions"
        ),
        format_noise_threshold(discovered.noise_threshold_milli)
    );
    if discovered.filtered_edges > 0 {
        println!(
            "{} removed by local infrequent-edge filtering",
            plural(discovered.filtered_edges, "relation", "relations")
        );
    }
    if discovered.used_flower_fallback {
        println!(
            "note: part of the log could not be structured by a cut; the model uses a permissive fallback there"
        );
    }
    println!("\nPROCESS TREE");
    print_process_tree(&discovered.tree, "  ");
    println!(
        "\nACCEPTING PETRI NET — {} places · {} transitions",
        discovered.net.place_count,
        discovered.net.transitions.len()
    );
    println!("Use --json for markings, silent transitions, and arcs.");
    Ok(ExitCode::SUCCESS)
}

fn parse_noise_threshold(value: &str) -> Result<u16, String> {
    parse_percent_threshold(value, "--noise-threshold")
}

fn parse_percent_threshold(value: &str, option: &str) -> Result<u16, String> {
    let value = value.trim().trim_end_matches('%');
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 1
    {
        return Err(format!(
            "{option} expects a percentage from 0 to 100 with at most one decimal, got '{value}'"
        ));
    }
    let whole: u16 = whole
        .parse()
        .map_err(|_| format!("{option} expects a percentage from 0 to 100, got '{value}'"))?;
    let tenth = fraction
        .bytes()
        .next()
        .map_or(0, |byte| u16::from(byte - b'0'));
    let milli = whole
        .checked_mul(10)
        .and_then(|scaled| scaled.checked_add(tenth))
        .ok_or_else(|| format!("{option} is too large: '{value}'"))?;
    if milli > 1000 {
        return Err(format!(
            "{option} expects a percentage from 0 to 100, got '{value}'"
        ));
    }
    Ok(milli)
}

fn format_noise_threshold(milli: u16) -> String {
    if milli.is_multiple_of(10) {
        (milli / 10).to_string()
    } else {
        format!("{}.{:01}", milli / 10, milli % 10)
    }
}

fn print_process_tree(tree: &LabelledProcessTree, indent: &str) {
    match tree {
        LabelledProcessTree::Activity { activity } => println!("{indent}{activity}"),
        LabelledProcessTree::Tau => println!("{indent}τ (silent)"),
        LabelledProcessTree::Sequence { children } => {
            println!("{indent}SEQUENCE");
            for child in children {
                print_process_tree(child, &format!("{indent}  "));
            }
        }
        LabelledProcessTree::Exclusive { children } => {
            println!("{indent}EXCLUSIVE CHOICE");
            for child in children {
                print_process_tree(child, &format!("{indent}  "));
            }
        }
        LabelledProcessTree::Parallel { children } => {
            println!("{indent}PARALLEL");
            for child in children {
                print_process_tree(child, &format!("{indent}  "));
            }
        }
        LabelledProcessTree::Loop { body, redo } => {
            println!("{indent}LOOP");
            println!("{indent}  BODY");
            print_process_tree(body, &format!("{indent}    "));
            println!("{indent}  REDO");
            print_process_tree(redo, &format!("{indent}    "));
        }
    }
}

fn analyze_command(args: &[String]) -> Result<ExitCode, String> {
    let mut path: Option<String> = None;
    let mut case = None;
    let mut activity = None;
    let mut timestamp = None;
    let mut policy = Policy::new();
    let mut json = false;
    let mut want_brief = false;
    let mut rules = Rules::new();
    let mut save_project: Option<String> = None;
    let mut open_project: Option<String> = None;
    let mut filter = Filter::new();
    let mut filter_explicit = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json = true,
            "--brief" => want_brief = true,
            "--case" => case = Some(take(args, &mut i, "--case")?),
            "--activity" => activity = Some(take(args, &mut i, "--activity")?),
            "--timestamp" => timestamp = Some(take(args, &mut i, "--timestamp")?),
            "--answer" => {
                let value = take(args, &mut i, "--answer")?;
                let (kind, option) = value
                    .split_once('=')
                    .ok_or_else(|| format!("--answer expects <kind>=<option>, got '{value}'"))?;
                policy = policy.answer(parse_kind(kind)?, option.to_owned());
            }
            "--rename" => {
                let value = take(args, &mut i, "--rename")?;
                let (from, to) = value
                    .split_once('=')
                    .ok_or_else(|| format!("--rename expects <old>=<new>, got '{value}'"))?;
                rules = rules.rename(from.trim(), to.trim());
            }
            "--exclude-activity" => {
                let value = take(args, &mut i, "--exclude-activity")?;
                rules = rules.exclude_activity(value.trim());
            }
            "--exclude-case" => {
                let value = take(args, &mut i, "--exclude-case")?;
                rules = rules.exclude_case(value.trim());
            }
            "--min-events" => {
                let value = take(args, &mut i, "--min-events")?;
                let n = value
                    .parse()
                    .map_err(|_| format!("--min-events expects a number, got '{value}'"))?;
                rules = rules.min_events_per_case(n);
            }
            "--drop-incomplete" => rules = rules.drop_incomplete_cases(true),
            "--save-project" => save_project = Some(take(args, &mut i, "--save-project")?),
            "--project" => open_project = Some(take(args, &mut i, "--project")?),
            "--rules" => {
                let path = take(args, &mut i, "--rules")?;
                let loaded = Rules::from_file(&path).map_err(|e| e.to_string())?;
                // A rules file is the base; individual flags refine it.
                rules = merge_rules(loaded, rules);
            }
            other => {
                if parse_filter_flag(other, args, &mut i, &mut filter)? {
                    filter_explicit = true;
                } else if other.starts_with('-') {
                    return Err(format!("unknown option '{other}'"));
                } else {
                    path = Some(other.to_owned());
                }
            }
        }
        i += 1;
    }

    // Reopening a project restores the mapping, the answers, and the rules,
    // then reports whether the source still matches what was recorded.
    let mut saved: Option<Project> = None;
    let mut current_source: Option<Source> = None;
    if let Some(project_path) = &open_project {
        let project = Project::load(project_path).map_err(|e| e.to_string())?;
        if path.is_none() {
            path = Some(project.source.path.clone());
        }
        let fingerprint =
            Source::fingerprint(path.as_deref().unwrap_or_default()).map_err(|e| e.to_string())?;
        report_source_state(&project, &fingerprint);
        current_source = Some(fingerprint);
        policy = merge_policies(project.policy.clone(), policy);
        rules = merge_rules(project.rules.clone(), rules);
        if !filter_explicit {
            filter = project.view_filter.clone();
        }
        if mapping_flags_absent(&case, &activity, &timestamp) {
            case = Some(project.mapping.case_id.clone());
            activity = Some(project.mapping.activity.clone());
            timestamp = Some(project.mapping.timestamp.clone());
        }
        saved = Some(project);
    }

    let path = path.ok_or("analyze needs a file. Try `glasstarn analyze orders.csv`.")?;

    let mapping = match (case, activity, timestamp) {
        (Some(c), Some(a), Some(t)) => {
            // Optional mappings survive a reopen; flags only cover the three
            // required fields.
            let mut m = Mapping::new(&c, &a, &t);
            if let Some(project) = &saved {
                m.event_order = project.mapping.event_order.clone();
                m.end_timestamp = project.mapping.end_timestamp.clone();
                m.resource = project.mapping.resource.clone();
                m.lifecycle = project.mapping.lifecycle.clone();
                m.cost = project.mapping.cost.clone();
                m.case_attributes = project.mapping.case_attributes.clone();
            }
            Some(m)
        }
        (None, None, None) => None,
        _ => {
            return Err(
                "specify all three of --case, --activity and --timestamp, or none of them"
                    .to_owned(),
            );
        }
    };

    let cache_lookup = open_project.as_ref().and_then(|project_path| {
        mapping
            .as_ref()
            .zip(current_source.as_ref())
            .map(|(mapping, source)| {
                let cache = AnalysisCache::beside(project_path);
                let key = CacheKey::new(
                    &source.path,
                    &source.sha256,
                    mapping,
                    &policy,
                    &rules,
                    &filter,
                );
                (cache, key)
            })
    });
    let cached = cache_lookup
        .as_ref()
        .and_then(|(cache, key)| cache.load(key));
    let cache_hit = cached.is_some();
    let outcome = match cached {
        Some(analysis) => {
            eprintln!("cache hit");
            Outcome::Complete(Box::new(analysis))
        }
        None => analyze_filtered(
            &path,
            mapping.clone(),
            &policy,
            rules.clone(),
            filter.clone(),
        )
        .map_err(|e| e.to_string())?,
    };

    if !cache_hit
        && let (Some((cache, key)), Outcome::Complete(analysis)) = (&cache_lookup, &outcome)
        && let Err(error) = cache.store(key, analysis)
    {
        eprintln!("warning: could not write analysis cache: {error}");
    }

    if let Some(target) = &save_project {
        // A wide file is unpivoted before mapping, so its mapping names
        // synthetic columns that do not exist in the source. Saving it would
        // produce a project that cannot reopen — better to refuse than to
        // write something broken.
        if let Outcome::Complete(a) = &outcome
            && a.unpivoted_from.is_some()
        {
            return Err(
                "this file has one row per case and was unpivoted automatically, so its mapping \
                 refers to columns that do not exist in the source. Saving a project for it is \
                 not supported yet, because the unpivot specification would have to be saved \
                 too."
                    .to_owned(),
            );
        }

        // The *resolved* mapping, with optional columns and case attributes —
        // not a reconstruction from the three required fields, which would
        // silently drop everything else.
        let resolved = match &outcome {
            Outcome::Complete(a) => a.mapping.clone(),
            Outcome::Blocked(b) => b.mapping.clone(),
        };
        let mut project = Project::new(&path, resolved, policy.clone(), rules.clone())
            .map_err(|e| e.to_string())?;
        project.view_filter = filter.clone();
        project.save(target).map_err(|e| e.to_string())?;
        if let Outcome::Complete(analysis) = &outcome {
            let cache = AnalysisCache::beside(target);
            let key = CacheKey::new(
                &project.source.path,
                &project.source.sha256,
                &project.mapping,
                &project.policy,
                &project.rules,
                &filter,
            );
            if let Err(error) = cache.store(&key, analysis) {
                eprintln!("warning: could not write analysis cache: {error}");
            }
        }
        println!("saved project to {target}\n");
    }

    match outcome {
        Outcome::Blocked(blocked) => {
            print_blocked(&blocked.report, json);
            // A distinct exit code: this is not a failure, it is a question.
            Ok(ExitCode::from(3))
        }
        Outcome::Complete(mut analysis) => {
            // A workbook read through a converted table is named as given.
            if let Some(original) = ORIGINAL_FILE.get() {
                analysis.source = original.clone();
            }
            if want_brief {
                print!("{}", brief(&analysis));
            } else if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&analysis).map_err(|e| e.to_string())?
                );
            } else {
                print_analysis(&analysis);
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Reports whether a reopened project's source still matches.
fn report_source_state(project: &Project, source: &Source) {
    let state = project.check_fingerprint(source);
    if !project.name.is_empty() {
        println!("project: {}", project.name);
    }
    if !state.reproduces() {
        println!("WARNING  {}", state.explain());
    }
    if project.engine_changed() {
        println!(
            "note     saved by engine {} — this is {}",
            project.engine_version,
            pm_project::ENGINE_VERSION
        );
    }
}

/// Whether none of the three mapping flags were given.
fn mapping_flags_absent(
    case: &Option<String>,
    activity: &Option<String>,
    timestamp: &Option<String>,
) -> bool {
    case.is_none() && activity.is_none() && timestamp.is_none()
}

/// Combines a saved policy with answers given as flags. Flags win.
fn merge_policies(base: Policy, flags: Policy) -> Policy {
    let mut out = base;
    for kind in [
        Kind::BlankTimestamps,
        Kind::DuplicateRows,
        Kind::AmbiguousDateOrder,
        Kind::MixedZoneAndNaive,
        Kind::NearDuplicateLabels,
    ] {
        if let Some(answer) = flags.get(&kind) {
            out = out.answer(kind, answer);
        }
    }
    out
}

fn take(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn parse_kind(text: &str) -> Result<Kind, String> {
    Ok(match text {
        "blank_timestamps" => Kind::BlankTimestamps,
        "duplicate_rows" => Kind::DuplicateRows,
        "ambiguous_date_order" => Kind::AmbiguousDateOrder,
        "mixed_zone" => Kind::MixedZoneAndNaive,
        "near_duplicate_labels" => Kind::NearDuplicateLabels,
        other => return Err(format!("unknown decision '{other}'")),
    })
}

fn kind_flag(kind: &Kind) -> &'static str {
    match kind {
        Kind::BlankTimestamps => "blank_timestamps",
        Kind::DuplicateRows => "duplicate_rows",
        Kind::AmbiguousDateOrder => "ambiguous_date_order",
        Kind::MixedZoneAndNaive => "mixed_zone",
        Kind::NearDuplicateLabels => "near_duplicate_labels",
        _ => "unknown",
    }
}

fn print_blocked(report: &pm_ingest::Report, json: bool) {
    if json {
        // Machine-readable enough to drive a UI, without pretending a JSON
        // schema for findings is settled yet.
        println!(
            "{{\"blocked\": true, \"decisions\": {}}}",
            report.decisions().len()
        );
        return;
    }

    println!("This log needs a decision before it can be analysed.\n");
    println!("Some things in real data have no correct answer that software can work out.");
    println!("Rather than guess and quietly produce wrong numbers, here is what was found:\n");

    for finding in report.decisions() {
        let decision = finding.decision.as_ref().expect("blocking finding");
        println!("  {}", finding.kind.title().to_uppercase());
        println!("  {}\n", finding.detail);
        println!("  {}\n", decision.question);
        for option in &decision.options {
            println!("    --answer {}={}", kind_flag(&finding.kind), option.id);
            println!("      {}", option.label);
            println!("      {}\n", option.effect);
        }
        println!("  Why this matters: {}\n", decision.risk);
        if !finding.examples.is_empty() {
            println!("  From your data:");
            for example in finding.examples.iter().take(3) {
                println!("    {example}");
            }
            println!();
        }
    }
}

fn print_analysis(a: &Analysis) {
    println!("{}", a.source);
    if a.view.narrowed {
        println!(
            "FILTERED VIEW — showing {} of {} cases ({:.0}%). \
             Everything below describes only those cases.",
            a.view.cases_shown, a.view.cases_total, a.view.percent_shown
        );
    }
    println!(
        "{} · {} · {} · {}",
        plural(a.summary.events, "event", "events"),
        plural(a.summary.cases, "case", "cases"),
        plural(a.summary.activities, "activity", "activities"),
        plural(a.summary.variants, "variant", "variants"),
    );
    if let Some(columns) = &a.unpivoted_from {
        println!(
            "wide table unpivoted — {} timestamp columns became activities:",
            columns.len()
        );
        for c in columns {
            println!("  {c}");
        }
    }
    if let Some(f) = &a.timestamp_format {
        println!(
            "mapped {} / {} / {} · timestamps read as {}",
            a.case_column, a.activity_column, a.timestamp_column, f
        );
    }
    if !a.case_attributes.is_empty() {
        println!(
            "{} kept as case attributes: {}",
            a.case_attributes.len(),
            a.case_attributes.join(", ")
        );
    }
    if !a.optional_columns.is_empty() {
        let extra: Vec<String> = a
            .optional_columns
            .iter()
            .map(|(field, column)| format!("{field}: {column}"))
            .collect();
        println!("also mapped {}", extra.join(" · "));
    }
    if let Some(cost) = a.summary.total_cost_microunits {
        println!(
            "total mapped cost: {} source currency units",
            cost_units(cost)
        );
    }

    println!("\nHOW CONCENTRATED IS THIS PROCESS?");
    println!(
        "  top 5 paths cover {:.0}% of cases",
        a.summary.top_five_coverage_percent
    );
    println!(
        "  {} {} for 80% of cases",
        plural(a.summary.variants_for_80_percent, "path", "paths"),
        if a.summary.variants_for_80_percent == 1 {
            "accounts"
        } else {
            "account"
        }
    );
    if a.summary.unique_variants > 0 {
        println!(
            "  {} paths were followed by exactly one case",
            a.summary.unique_variants
        );
    }

    println!("\nPROCESS MAP — FREQUENCY OVERLAY");
    let mut frequency_edges: Vec<&pm_query::Edge> = a.edges.iter().collect();
    frequency_edges.sort_by_key(|edge| edge.frequency_rank);
    if frequency_edges.is_empty() {
        println!("  no transitions — every case has a single event");
    } else {
        for edge in frequency_edges.into_iter().take(5) {
            println!(
                "  {:>7}  {:>5.1}% of transitions   {} → {}   ({} affected)",
                plural(edge.frequency as usize, "time", "times"),
                edge.frequency_share_percent,
                edge.from,
                edge.to,
                plural(edge.case_frequency as usize, "case", "cases")
            );
        }
    }

    println!("\nPROCESS MAP — PERFORMANCE / BOTTLENECK RANKING");
    if !a.measures_true_waiting {
        println!(
            "  (start-to-start elapsed time — includes each activity's own duration,\n\
             \x20  because the log has one timestamp per event. Map an end timestamp\n\
             \x20  to separate waiting from processing.)"
        );
    }
    if a.bottlenecks.is_empty() {
        println!("  no transitions were eligible for duration measurement");
    } else {
        for bottleneck in a.bottlenecks.iter().take(5) {
            println!(
                "  #{:<2} {:>6} avg  {:>5.1}% of total   {} → {}   ({}/{} timed)",
                bottleneck.rank,
                duration(bottleneck.mean_seconds),
                bottleneck.share_percent,
                bottleneck.from,
                bottleneck.to,
                bottleneck.duration_eligible_occurrences,
                bottleneck.total_occurrences
            );
        }
    }

    if a.rework.cases_affected > 0 {
        println!("\nWHAT GETS REDONE");
        println!(
            "  {} affected ({:.1}% of cases) · {} immediate {} · {} {} after intervening work",
            plural(a.rework.cases_affected as usize, "case", "cases"),
            a.rework.case_percent,
            a.rework.immediate_repetitions,
            if a.rework.immediate_repetitions == 1 {
                "repetition"
            } else {
                "repetitions"
            },
            a.rework.returns_after_intervening,
            if a.rework.returns_after_intervening == 1 {
                "return"
            } else {
                "returns"
            }
        );
        for activity in a.rework.activities.iter().take(5) {
            println!(
                "  {:>7}  {}   ({} affected; e.g. {})",
                plural(activity.repetitions as usize, "repeat", "repeats"),
                activity.activity,
                plural(activity.cases_affected as usize, "case", "cases"),
                activity.example_cases.join(", ")
            );
        }
        println!("  observed loop paths:");
        for loop_path in a.rework.loops.iter().take(5) {
            println!(
                "  {:>7}  {}   (avg {} from {}/{} timed; e.g. {})",
                plural(loop_path.occurrences as usize, "time", "times"),
                loop_path.sequence.join(" → "),
                if loop_path.duration_eligible_occurrences == 0 {
                    "unavailable".to_owned()
                } else {
                    duration(loop_path.mean_duration_seconds)
                },
                loop_path.duration_eligible_occurrences,
                loop_path.occurrences,
                loop_path.example_cases.join(", ")
            );
        }
        if a.rework.omitted_loop_candidates > 0 {
            println!(
                "  {} oversized or excess loop-path candidates omitted from detail; activity counts remain complete",
                a.rework.omitted_loop_candidates
            );
        }
    }

    println!("\nMOST COMMON PATHS");
    for v in a.variants.iter().take(5) {
        println!(
            "  {:>4} {} ({:>4.1}%)  avg {:>11} ({}/{} timed)   {}",
            v.cases,
            if v.cases == 1 { "case " } else { "cases" },
            v.percent,
            if v.duration_eligible_cases == 0 {
                "unavailable".to_owned()
            } else {
                duration(v.mean_throughput_seconds)
            },
            v.duration_eligible_cases,
            v.cases,
            v.sequence.join(" → ")
        );
        if !v.example_cases.is_empty() {
            println!("       e.g. {}", v.example_cases.join(", "));
        }
    }

    println!("\nTHROUGHPUT");
    if a.summary.duration_eligible_cases == 0 {
        println!("  mean   unavailable");
        println!("  median unavailable");
    } else {
        println!("  mean   {}", duration(a.summary.mean_throughput_seconds));
        println!("  median {}", duration(a.summary.median_throughput_seconds));
    }
    println!(
        "  {} of {} cases eligible for duration statistics",
        a.summary.duration_eligible_cases, a.summary.cases
    );
    if a.summary.single_event_cases > 0 {
        println!(
            "  {} cases have a single event, so their throughput is zero",
            a.summary.single_event_cases
        );
    }

    if let Some(org) = &a.organization {
        println!("\nWHO PASSES WORK TO WHOM");
        println!(
            "  {} · {:.0}% of consecutive steps changed hands",
            plural(org.resources, "resource", "resources"),
            org.fragmentation_percent
        );
        for h in org.handovers.iter().take(5) {
            println!(
                "  {:>6} avg  {:>5.1}% of handover time   {} → {}   ({}; {}/{} timed)",
                duration(h.mean_seconds),
                h.share_percent,
                h.from,
                h.to,
                plural(h.occurrences as usize, "time", "times"),
                h.duration_eligible_occurrences,
                h.occurrences
            );
        }
        println!("  Waiting time belongs to the transition, not to a person.");
    }

    if a.excluded_events > 0 || a.excluded_cases > 0 {
        println!("\nEXCLUDED BY YOUR RULES");
        if a.excluded_events > 0 {
            println!("  {} events", a.excluded_events);
        }
        if a.excluded_cases > 0 {
            println!("  {} cases", a.excluded_cases);
        }
    }

    if !a.notes.is_empty() {
        println!("\nWORTH KNOWING");
        for note in &a.notes {
            println!("  - {note}");
        }
    }

    if !a.offers.is_empty() {
        println!("\nYOU CAN CHANGE THIS");
        for offer in &a.offers {
            println!("  {}", offer.question);
            for option in &offer.options {
                println!("    --answer {}={}", offer.flag, option.0);
                println!("      {}", option.1);
            }
        }
    }

    println!(
        "\nThe map above is a directly-follows graph. It shows what followed what,\n\
         not a validated process model — concurrent steps appear as if they loop.\n\
         The path list is the honest account of what actually happened."
    );
}

/// Combines a loaded rule set with rules given as flags.
///
/// Flags win, so a file can be refined on the command line without editing it.
fn merge_rules(base: Rules, flags: Rules) -> Rules {
    let mut out = base;
    for (from, to) in flags.rename {
        out.rename.insert(from, to);
    }
    out.exclude_activities.extend(flags.exclude_activities);
    out.exclude_cases.extend(flags.exclude_cases);
    if flags.min_events_per_case > 0 {
        out.min_events_per_case = flags.min_events_per_case;
    }
    if flags.drop_incomplete_cases {
        out.drop_incomplete_cases = true;
    }
    if flags.observation_window.is_some() {
        out.observation_window = flags.observation_window;
    }
    out
}

/// Parses a duration such as `2d`, `6h`, `30m`, or `45s`.
fn parse_duration(text: &str) -> Result<i64, String> {
    let text = text.trim();
    let (value, unit) = text.split_at(
        text.find(|c: char| c.is_alphabetic())
            .ok_or_else(|| format!("'{text}' needs a unit, e.g. 2d, 6h, 30m"))?,
    );
    let value: f64 = value
        .parse()
        .map_err(|_| format!("'{text}' is not a duration, e.g. 2d, 6h, 30m"))?;
    let seconds = match unit.to_lowercase().as_str() {
        "s" | "sec" | "secs" => 1.0,
        "m" | "min" | "mins" => 60.0,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3_600.0,
        "d" | "day" | "days" => 86_400.0,
        "w" | "week" | "weeks" => 604_800.0,
        other => {
            return Err(format!(
                "unknown duration unit '{other}'; use s, m, h, d or w"
            ));
        }
    };
    Ok((value * seconds * 1_000_000.0) as i64)
}

/// Renders a count with the right singular or plural noun.
fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Formats seconds as a compact human duration.
fn duration(seconds: i64) -> String {
    let s = seconds.abs();
    let out = if s >= 86_400 {
        format!("{:.1}d", s as f64 / 86_400.0)
    } else if s >= 3_600 {
        format!("{:.1}h", s as f64 / 3_600.0)
    } else if s >= 60 {
        format!("{:.0}m", s as f64 / 60.0)
    } else {
        format!("{s}s")
    };
    if seconds < 0 { format!("-{out}") } else { out }
}

fn cost_units(microunits: i128) -> String {
    let sign = if microunits < 0 { "-" } else { "" };
    let absolute = microunits.abs();
    let whole = absolute / 1_000_000;
    let fraction = absolute % 1_000_000;
    if fraction == 0 {
        format!("{sign}{whole}")
    } else {
        let fraction = format!("{fraction:06}");
        format!("{sign}{whole}.{}", fraction.trim_end_matches('0'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_rejects_invalid_options_before_binding() {
        assert!(serve_command(&["--unknown".to_owned()]).is_err());
        assert!(serve_command(&["--port".to_owned(), "not-a-port".to_owned()]).is_err());
        assert!(serve_command(&["--port".to_owned()]).is_err());
        assert!(serve_command(&["--agent-access".to_owned()]).is_err());
        assert!(serve_command(&["--project".to_owned()]).is_err());
        assert!(serve_command(&["--project".to_owned(), "../escape".to_owned()]).is_err());
    }

    #[test]
    fn live_mcp_requires_an_explicit_reviewed_policy() {
        let error = mcp_command(&["--live-port".to_owned(), "43127".to_owned()]).unwrap_err();
        assert!(error.contains("requires an explicit --exposure-policy"));
        assert!(
            mcp_command(&["--live-port".to_owned(), "not-a-port".to_owned()])
                .unwrap_err()
                .contains("expects a number")
        );
        assert!(
            mcp_command(&["--live-port".to_owned(), "0".to_owned()])
                .unwrap_err()
                .contains("1 to 65535")
        );
        assert!(
            mcp_command(&[
                "--live-port".to_owned(),
                "43127".to_owned(),
                "--project-dir".to_owned(),
                ".".to_owned(),
                "--exposure-policy".to_owned(),
                "not-read.json".to_owned(),
            ])
            .unwrap_err()
            .contains("cannot be combined")
        );
    }

    #[test]
    fn browser_launcher_passes_the_authenticated_url_as_one_argument() {
        let url = "http://127.0.0.1:4321/#token=abc123";
        let (_program, args) = browser_command(url);
        assert_eq!(args.last().map(String::as_str), Some(url));
        assert_eq!(
            args.iter()
                .filter(|argument| argument.contains(url))
                .count(),
            1
        );
    }

    #[test]
    fn formats_exact_costs_without_a_currency_guess() {
        assert_eq!(cost_units(12_625_000), "12.625");
        assert_eq!(cost_units(-500_000), "-0.5");
    }

    #[test]
    fn shared_filter_parser_carries_view_settings_into_drilldown() {
        let args = vec![
            "--starts-with".to_owned(),
            "Receive".to_owned(),
            "--ends-with".to_owned(),
            "Archive".to_owned(),
            "--directly-follows".to_owned(),
            "Receive".to_owned(),
            "Review".to_owned(),
            "--eventually-follows".to_owned(),
            "Review".to_owned(),
            "Archive".to_owned(),
            "--where".to_owned(),
            "region=EMEA".to_owned(),
            "--complete-only".to_owned(),
            "--slower-than".to_owned(),
            "2h".to_owned(),
        ];
        let mut filter = Filter::new();
        let mut i = 0;
        while i < args.len() {
            let flag = args[i].clone();
            assert!(parse_filter_flag(&flag, &args, &mut i, &mut filter).unwrap());
            i += 1;
        }
        assert_eq!(filter.starts_with.as_deref(), Some("Receive"));
        assert_eq!(filter.ends_with.as_deref(), Some("Archive"));
        assert!(
            filter
                .directly_follows
                .contains(&("Receive".to_owned(), "Review".to_owned()))
        );
        assert!(
            filter
                .eventually_follows
                .contains(&("Review".to_owned(), "Archive".to_owned()))
        );
        assert_eq!(
            filter.attribute_equals,
            vec![("region".to_owned(), "EMEA".to_owned())]
        );
        assert!(filter.complete_only);
        assert_eq!(filter.min_duration, Some(2 * 3_600_000_000));
    }

    #[test]
    fn trace_pair_flags_require_both_activities() {
        for flag in ["--directly-follows", "--eventually-follows"] {
            let args = vec![flag.to_owned(), "Receive".to_owned()];
            let mut filter = Filter::new();
            let mut i = 0;
            let error = parse_filter_flag(flag, &args, &mut i, &mut filter).unwrap_err();
            assert!(error.contains(&format!("{flag} needs a value")), "{error}");
            assert!(filter.is_empty());
        }
    }

    #[test]
    fn a_saved_filter_is_restored_unless_view_flags_override_it() {
        let prep = Prep {
            project_filter: Some(Filter::new().min_events(4)),
            ..Prep::default()
        };
        assert_eq!(
            resolve_filter(&prep, false, Filter::new()).min_events,
            Some(4)
        );
        assert_eq!(
            resolve_filter(&prep, true, Filter::new().min_events(2)).min_events,
            Some(2)
        );
    }

    #[test]
    fn a_case_excluded_from_the_analysis_view_is_not_returned_as_evidence() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/canonical/01-sequential.csv")
            .display()
            .to_string();
        let error = case_command(&[
            path,
            "ORDER-1".to_owned(),
            "--without-activity".to_owned(),
            "Ship Order".to_owned(),
        ])
        .expect_err("ORDER-1 is outside this filtered view");
        assert!(error.contains("current view"));
    }

    #[test]
    fn flags_refine_a_loaded_rule_set() {
        let base = Rules::new().rename("A", "Alpha").exclude_activity("Noise");
        let flags = Rules::new().rename("B", "Beta").min_events_per_case(3);
        let merged = merge_rules(base, flags);
        assert_eq!(merged.resolve_activity("A"), Some("Alpha"));
        assert_eq!(merged.resolve_activity("B"), Some("Beta"));
        assert_eq!(merged.resolve_activity("Noise"), None);
        assert_eq!(merged.min_events_per_case, 3);
    }

    #[test]
    fn a_flag_overrides_the_same_rule_from_a_file() {
        let base = Rules::new().rename("A", "FromFile");
        let flags = Rules::new().rename("A", "FromFlag");
        assert_eq!(
            merge_rules(base, flags).resolve_activity("A"),
            Some("FromFlag")
        );
    }

    /// Case totals render through the same exact path as the whole-log total:
    /// no float conversion, no overflow, and no invented precision.
    #[test]
    fn renders_case_costs_exactly() {
        assert_eq!(cost_units(1_500_000), "1.5");
        assert_eq!(cost_units(7_262_250_000), "7262.25");
        assert_eq!(cost_units(0), "0");
        assert_eq!(cost_units(-2_500_000), "-2.5");
        // A value a float would round; exact arithmetic keeps every digit.
        assert_eq!(cost_units(1), "0.000001");
        // Beyond i64 entirely, which is why totals accumulate in i128.
        assert_eq!(
            cost_units(i128::from(i64::MAX) * 3),
            "27670116110564.327421"
        );
    }

    #[test]
    fn case_requires_a_file_and_an_id() {
        assert!(case_command(&[]).is_err());
        assert!(case_command(&["orders.csv".to_owned()]).is_err());
    }

    #[test]
    fn quality_requires_a_file_or_project() {
        assert!(quality_command(&[]).is_err());
    }

    #[test]
    fn parses_repeatable_object_centric_filters() {
        let parsed = parse_ocel_args(&[
            "log.jsonocel".to_owned(),
            "--object-type".to_owned(),
            "item".to_owned(),
            "--object-type".to_owned(),
            "package".to_owned(),
            "--e2o-qualifier".to_owned(),
            "packed item".to_owned(),
            "--o2o-qualifier".to_owned(),
            "part of".to_owned(),
            "--bottlenecks".to_owned(),
            "--json".to_owned(),
        ])
        .unwrap();
        assert_eq!(parsed.path, "log.jsonocel");
        assert_eq!(parsed.filter.object_types, ["item", "package"]);
        assert_eq!(parsed.filter.event_object_qualifiers, ["packed item"]);
        assert_eq!(parsed.filter.object_object_qualifiers, ["part of"]);
        assert!(parsed.json);
        assert!(parsed.bottlenecks);
        assert_eq!(parsed.project_as, None);
        assert_eq!(parsed.case_limit, 5);
        assert_eq!(parsed.event_limit, 100);
    }

    #[test]
    fn object_centric_analysis_runs_through_the_real_cli_path() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/ocel/order-management.jsonocel")
            .display()
            .to_string();
        assert_eq!(
            ocel_command(&[
                source,
                "--object-type".to_owned(),
                "package".to_owned(),
                "--json".to_owned(),
            ])
            .unwrap(),
            ExitCode::SUCCESS
        );
    }

    #[test]
    fn object_centric_xml_runs_through_the_real_cli_path() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/ocel/order-management.xmlocel")
            .display()
            .to_string();
        assert_eq!(
            ocel_command(&[source, "--json".to_owned()]).unwrap(),
            ExitCode::SUCCESS
        );
    }

    #[test]
    fn object_centric_sqlite_runs_through_the_real_cli_path() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/ocel/order-management.sqlite")
            .display()
            .to_string();
        assert_eq!(
            ocel_command(&[source, "--json".to_owned()]).unwrap(),
            ExitCode::SUCCESS
        );
    }

    #[test]
    fn object_centric_bottlenecks_run_through_the_real_cli_path() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/ocel/order-management.jsonocel")
            .display()
            .to_string();
        assert_eq!(
            ocel_command(&[source.clone(), "--bottlenecks".to_owned(),]).unwrap(),
            ExitCode::SUCCESS
        );
        assert_eq!(
            ocel_command(&[
                source,
                "--object-type".to_owned(),
                "package".to_owned(),
                "--bottlenecks".to_owned(),
                "--json".to_owned(),
            ])
            .unwrap(),
            ExitCode::SUCCESS
        );
    }

    #[test]
    fn object_centric_projection_runs_through_the_real_cli_path() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/ocel/order-management.jsonocel")
            .display()
            .to_string();
        assert_eq!(
            ocel_command(&[
                source.clone(),
                "--project-as".to_owned(),
                "item".to_owned(),
                "--case-limit".to_owned(),
                "1".to_owned(),
                "--event-limit".to_owned(),
                "1".to_owned(),
            ])
            .unwrap(),
            ExitCode::SUCCESS
        );
        assert_eq!(
            ocel_command(&[
                source,
                "--project-as".to_owned(),
                "package".to_owned(),
                "--json".to_owned(),
            ])
            .unwrap(),
            ExitCode::SUCCESS
        );
    }

    #[test]
    fn object_centric_discovery_runs_through_the_real_cli_path() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/ocel/order-management.jsonocel")
            .display()
            .to_string();
        assert_eq!(
            ocel_command(&[
                source.clone(),
                "--discover-model".to_owned(),
                "--noise-threshold".to_owned(),
                "12.5".to_owned(),
                "--variable-arc-threshold".to_owned(),
                "90".to_owned(),
            ])
            .unwrap(),
            ExitCode::SUCCESS
        );
        assert_eq!(
            ocel_command(&[
                source,
                "--object-type".to_owned(),
                "package".to_owned(),
                "--discover-model".to_owned(),
                "--json".to_owned(),
            ])
            .unwrap(),
            ExitCode::SUCCESS
        );
    }

    #[test]
    fn object_centric_discovery_options_are_explicit_and_bounded() {
        let parsed = parse_ocel_args(&[
            "log.jsonocel".to_owned(),
            "--discover-model".to_owned(),
            "--noise-threshold".to_owned(),
            "0".to_owned(),
            "--variable-arc-threshold".to_owned(),
            "99.9%".to_owned(),
        ])
        .unwrap();
        assert!(parsed.discover_model);
        assert_eq!(parsed.discovery_config.noise_threshold_milli, 0);
        assert_eq!(parsed.discovery_config.variable_arc_threshold_milli, 999);
        assert!(
            parse_ocel_args(&[
                "log.jsonocel".to_owned(),
                "--noise-threshold".to_owned(),
                "20".to_owned(),
            ])
            .is_err()
        );
        assert!(
            parse_ocel_args(&[
                "log.jsonocel".to_owned(),
                "--discover-model".to_owned(),
                "--variable-arc-threshold".to_owned(),
                "100.1".to_owned(),
            ])
            .is_err()
        );
        assert!(
            parse_ocel_args(&[
                "log.jsonocel".to_owned(),
                "--discover-model".to_owned(),
                "--bottlenecks".to_owned(),
            ])
            .is_err()
        );
    }

    #[test]
    fn object_centric_conformance_runs_through_the_real_cli_path() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/ocel/order-management.jsonocel");
        let baseline = pm_query::discover_ocel_model(
            &source,
            &OcelFilter::default(),
            pm_core::OcelDiscoveryConfig::default(),
        )
        .unwrap();
        let model_path = std::env::temp_dir().join(format!(
            "open-process-ocel-model-{}.json",
            std::process::id()
        ));
        std::fs::write(&model_path, serde_json::to_vec(&baseline).unwrap()).unwrap();

        let result = ocel_command(&[
            source.display().to_string(),
            "--conformance-model".to_owned(),
            model_path.display().to_string(),
            "--detail-limit".to_owned(),
            "5".to_owned(),
            "--json".to_owned(),
        ]);
        let alignment = ocel_command(&[
            source.display().to_string(),
            "--conformance-model".to_owned(),
            model_path.display().to_string(),
            "--conformance-algorithm".to_owned(),
            "alignment".to_owned(),
            "--detail-limit".to_owned(),
            "5".to_owned(),
            "--json".to_owned(),
        ]);
        std::fs::remove_file(model_path).unwrap();
        assert_eq!(result.unwrap(), ExitCode::SUCCESS);
        assert_eq!(alignment.unwrap(), ExitCode::SUCCESS);
    }

    #[test]
    fn object_centric_conformance_options_are_explicit_and_bounded() {
        let parsed = parse_ocel_args(&[
            "log.jsonocel".to_owned(),
            "--conformance-model".to_owned(),
            "baseline.json".to_owned(),
            "--detail-limit".to_owned(),
            "250".to_owned(),
        ])
        .unwrap();
        assert_eq!(parsed.conformance_model.as_deref(), Some("baseline.json"));
        assert_eq!(parsed.detail_limit, 250);
        assert!(!parsed.conformance_alignment);
        let alignment = parse_ocel_args(&[
            "log.jsonocel".to_owned(),
            "--conformance-model".to_owned(),
            "baseline.json".to_owned(),
            "--conformance-algorithm".to_owned(),
            "alignment".to_owned(),
        ])
        .unwrap();
        assert!(alignment.conformance_alignment);
        assert!(
            parse_ocel_args(&[
                "log.jsonocel".to_owned(),
                "--detail-limit".to_owned(),
                "5".to_owned(),
            ])
            .is_err()
        );
        assert!(
            parse_ocel_args(&[
                "log.jsonocel".to_owned(),
                "--conformance-algorithm".to_owned(),
                "alignment".to_owned(),
            ])
            .is_err()
        );
        assert!(
            parse_ocel_args(&[
                "log.jsonocel".to_owned(),
                "--conformance-model".to_owned(),
                "baseline.json".to_owned(),
                "--detail-limit".to_owned(),
                "10001".to_owned(),
            ])
            .is_err()
        );
        assert!(
            parse_ocel_args(&[
                "log.jsonocel".to_owned(),
                "--conformance-model".to_owned(),
                "baseline.json".to_owned(),
                "--discover-model".to_owned(),
            ])
            .is_err()
        );
    }

    #[test]
    fn object_centric_projection_options_are_explicit_and_bounded() {
        assert!(
            parse_ocel_args(&[
                "log.jsonocel".to_owned(),
                "--bottlenecks".to_owned(),
                "--project-as".to_owned(),
                "item".to_owned(),
            ])
            .is_err()
        );
        assert!(
            parse_ocel_args(&[
                "log.jsonocel".to_owned(),
                "--case-limit".to_owned(),
                "2".to_owned(),
            ])
            .is_err()
        );
        assert!(
            parse_ocel_args(&[
                "log.jsonocel".to_owned(),
                "--project-as".to_owned(),
                "item".to_owned(),
                "--event-limit".to_owned(),
                "0".to_owned(),
            ])
            .is_err()
        );
    }

    #[test]
    fn object_centric_cli_rejects_missing_files_and_unknown_options() {
        assert!(parse_ocel_args(&[]).is_err());
        assert!(parse_ocel_args(&["log.jsonocel".to_owned(), "--wat".to_owned()]).is_err());
    }

    #[test]
    fn automation_targeting_runs_through_the_real_cli_path() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("automation")
                .replace("::", "-")
        );
        let log_path = std::env::temp_dir().join(format!("pm-automation-{suffix}.csv"));
        let rules_path = std::env::temp_dir().join(format!("pm-automation-{suffix}.json"));
        std::fs::write(
            &log_path,
            "case_id,activity,timestamp,resource\nC1,Receive,2026-01-01 09:00:00,SYSTEM\nC1,Review,2026-01-01 10:00:00,alice\n",
        )
        .unwrap();
        std::fs::write(
            &rules_path,
            r#"{"resource_kinds":{"SYSTEM":"system","alice":"human"}}"#,
        )
        .unwrap();
        let common = vec![
            log_path.display().to_string(),
            "--rules".to_owned(),
            rules_path.display().to_string(),
            "--json".to_owned(),
        ];
        assert_eq!(resources_command(&common).unwrap(), ExitCode::SUCCESS);
        assert_eq!(automation_command(&common).unwrap(), ExitCode::SUCCESS);
        std::fs::remove_file(log_path).unwrap();
        std::fs::remove_file(rules_path).unwrap();
    }

    #[test]
    fn cohort_comparison_runs_through_the_real_cli_path() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("comparison")
                .replace("::", "-")
        );
        let log_path = std::env::temp_dir().join(format!("pm-comparison-{suffix}.csv"));
        std::fs::write(
            &log_path,
            "case_id,activity,timestamp,region\n\
             E1,Start,2026-01-01 09:00:00,East\n\
             E1,Review,2026-01-01 10:00:00,East\n\
             W1,Start,2026-01-02 09:00:00,West\n\
             W1,Escalate,2026-01-02 12:00:00,West\n",
        )
        .unwrap();
        assert_eq!(
            compare_cohorts_command(&[
                log_path.display().to_string(),
                "--attribute".to_owned(),
                "region".to_owned(),
                "--left".to_owned(),
                "East".to_owned(),
                "--right".to_owned(),
                "West".to_owned(),
                "--json".to_owned(),
            ])
            .unwrap(),
            ExitCode::SUCCESS
        );
        std::fs::remove_file(log_path).unwrap();
    }

    #[test]
    fn where_reads_exact_values_and_number_bounds() {
        let filter = parse_where(Filter::new(), "amount>=50,000").unwrap();
        assert_eq!(filter.attribute_between[0].attribute, "amount");
        assert_eq!(filter.attribute_between[0].at_least, Some(50_000.0));
        let filter = parse_where(Filter::new(), "amount <= 100").unwrap();
        assert_eq!(filter.attribute_between[0].at_most, Some(100.0));
        let filter = parse_where(Filter::new(), "region=EMEA").unwrap();
        assert_eq!(filter.attribute_equals, [("region".into(), "EMEA".into())]);
        for bad in ["amount>50000", "amount<5", "amount>=lots", "region"] {
            assert!(parse_where(Filter::new(), bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn amounts_print_with_separators() {
        assert_eq!(amount(1_234_567.891), "1,234,567.89");
        assert_eq!(amount(50_000.0), "50,000");
        assert_eq!(amount(-999.5), "-999.50");
        assert_eq!(amount(0.0), "0");
    }

    #[test]
    fn values_run_through_the_real_cli_path() {
        let log_path = std::env::temp_dir().join(format!("pm-values-{}.csv", std::process::id()));
        std::fs::write(
            &log_path,
            "case_id,activity,timestamp,amount\n\
             I1,Receive,2026-01-01 09:00:00,75000\n\
             I1,Pay,2026-01-02 09:00:00,75000\n\
             I2,Receive,2026-01-01 09:00:00,900\n",
        )
        .unwrap();
        let run = |extra: &[&str]| {
            let mut args = vec![log_path.display().to_string()];
            args.extend(extra.iter().map(|arg| (*arg).to_owned()));
            values_command(&args)
        };
        assert_eq!(
            run(&[
                "--field",
                "amount",
                "--by",
                "last-step",
                "--where",
                "amount>=50000"
            ])
            .unwrap(),
            ExitCode::SUCCESS
        );
        assert_eq!(
            run(&["--field", "amount", "--json"]).unwrap(),
            ExitCode::SUCCESS
        );
        assert!(run(&["--field", "activity"]).is_err());
        assert!(run(&[]).unwrap_err().contains("--field"));
        std::fs::remove_file(log_path).unwrap();
    }

    #[test]
    fn cohort_comparison_requires_both_distinct_values() {
        assert!(compare_cohorts_command(&[]).is_err());
        assert!(
            compare_cohorts_command(&[
                "events.csv".to_owned(),
                "--attribute".to_owned(),
                "region".to_owned(),
                "--left".to_owned(),
                "East".to_owned(),
            ])
            .is_err()
        );
    }

    #[test]
    fn period_comparison_runs_through_the_real_cli_path() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("period-comparison")
                .replace("::", "-")
        );
        let log_path = std::env::temp_dir().join(format!("pm-period-comparison-{suffix}.csv"));
        std::fs::write(
            &log_path,
            "case_id,activity,timestamp\n\
             B1,Start,2026-01-01 09:00:00\n\
             B1,Review,2026-01-01 10:00:00\n\
             C1,Start,2026-02-01 09:00:00\n\
             C1,Escalate,2026-02-01 12:00:00\n",
        )
        .unwrap();
        let loaded = pm_query::load_log(&log_path, None, &Policy::new(), Rules::new())
            .unwrap()
            .unwrap();
        let starts = loaded
            .log
            .traces
            .iter()
            .map(|trace| trace.start())
            .collect::<Vec<_>>();
        assert_eq!(
            compare_periods_command(&[
                log_path.display().to_string(),
                "--baseline-start".to_owned(),
                starts[0].to_string(),
                "--baseline-end".to_owned(),
                starts[0].to_string(),
                "--comparison-start".to_owned(),
                starts[1].to_string(),
                "--comparison-end".to_owned(),
                starts[1].to_string(),
                "--json".to_owned(),
            ])
            .unwrap(),
            ExitCode::SUCCESS
        );
        std::fs::remove_file(log_path).unwrap();
    }

    #[test]
    fn period_comparison_rejects_missing_or_non_numeric_boundaries() {
        assert!(compare_periods_command(&[]).is_err());
        assert!(
            compare_periods_command(&[
                "events.csv".to_owned(),
                "--baseline-start".to_owned(),
                "yesterday".to_owned(),
            ])
            .is_err()
        );
    }

    #[test]
    fn conformance_requires_an_explicit_model() {
        assert!(conformance_command(&[]).is_err());
        assert!(conformance_command(&["orders.csv".to_owned()]).is_err());
    }

    #[test]
    fn conformance_runs_against_a_saved_discovery_model() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/canonical/01-sequential.csv")
            .display()
            .to_string();
        let loaded = pm_query::load_log(&source, None, &Policy::new(), Rules::new())
            .unwrap()
            .unwrap();
        let model = pm_query::discover_formal(
            &source,
            &loaded.log,
            &Filter::new(),
            pm_core::InductiveMinerConfig::default(),
        )
        .unwrap();
        let model_path = std::env::temp_dir().join(format!(
            "open-process-cli-model-{}-{}.json",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .replace("::", "-")
        ));
        std::fs::write(&model_path, serde_json::to_vec(&model).unwrap()).unwrap();
        let result = conformance_command(&[
            source.clone(),
            "--model".to_owned(),
            model_path.display().to_string(),
            "--limit".to_owned(),
            "1".to_owned(),
        ]);
        let alignment_result = conformance_command(&[
            source,
            "--model".to_owned(),
            model_path.display().to_string(),
            "--algorithm".to_owned(),
            "alignment".to_owned(),
            "--json".to_owned(),
        ]);
        std::fs::remove_file(model_path).unwrap();
        assert_eq!(result.unwrap(), ExitCode::SUCCESS);
        assert_eq!(alignment_result.unwrap(), ExitCode::SUCCESS);
    }

    #[test]
    fn imported_bpmn_flows_into_conformance() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/canonical/01-sequential.csv")
            .display()
            .to_string();
        let bpmn_path =
            std::env::temp_dir().join(format!("open-process-cli-bpmn-{}.bpmn", std::process::id()));
        let model_path = std::env::temp_dir().join(format!(
            "open-process-cli-bpmn-model-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &bpmn_path,
            r#"<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"><process id="order">
<startEvent id="s"/><task id="receive" name="Receive Order"/><task id="check" name="Check Credit"/>
<task id="approve" name="Approve Order"/><task id="ship" name="Ship Order"/><endEvent id="e"/>
<sequenceFlow id="f1" sourceRef="s" targetRef="receive"/><sequenceFlow id="f2" sourceRef="receive" targetRef="check"/>
<sequenceFlow id="f3" sourceRef="check" targetRef="approve"/><sequenceFlow id="f4" sourceRef="approve" targetRef="ship"/>
<sequenceFlow id="f5" sourceRef="ship" targetRef="e"/></process></definitions>"#,
        )
        .unwrap();
        assert_eq!(
            import_bpmn_command(&[bpmn_path.display().to_string()]).unwrap(),
            ExitCode::SUCCESS
        );
        let model = pm_query::import_bpmn_model(&bpmn_path).unwrap();
        std::fs::write(&model_path, serde_json::to_vec(&model).unwrap()).unwrap();
        let result = conformance_command(&[
            source,
            "--model".to_owned(),
            model_path.display().to_string(),
        ]);
        std::fs::remove_file(bpmn_path).unwrap();
        std::fs::remove_file(model_path).unwrap();
        assert_eq!(result.unwrap(), ExitCode::SUCCESS);
    }

    #[test]
    fn parses_noise_thresholds_exactly() {
        assert_eq!(parse_noise_threshold("0").unwrap(), 0);
        assert_eq!(parse_noise_threshold("12.5").unwrap(), 125);
        assert_eq!(parse_noise_threshold("20%").unwrap(), 200);
        assert_eq!(parse_noise_threshold("100.0").unwrap(), 1000);
        assert!(parse_noise_threshold("100.1").is_err());
        assert!(parse_noise_threshold("12.55").is_err());
        assert!(parse_noise_threshold("noisy").is_err());
    }

    #[test]
    fn discovery_runs_through_the_real_ingestion_path() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/canonical/01-sequential.csv")
            .display()
            .to_string();
        assert_eq!(
            discover_command(&[
                source.clone(),
                "--noise-threshold".to_owned(),
                "12.5".to_owned(),
            ])
            .unwrap(),
            ExitCode::SUCCESS
        );
        assert_eq!(
            discover_command(&[source.clone(), "--json".to_owned()]).unwrap(),
            ExitCode::SUCCESS
        );
        assert_eq!(
            discover_command(&[
                source.clone(),
                "--algorithm".to_owned(),
                "alpha".to_owned(),
                "--json".to_owned(),
            ])
            .unwrap(),
            ExitCode::SUCCESS
        );
        assert_eq!(
            discover_command(&[
                source,
                "--algorithm".to_owned(),
                "heuristics".to_owned(),
                "--json".to_owned(),
            ])
            .unwrap(),
            ExitCode::SUCCESS
        );
    }

    #[test]
    fn cases_requires_something_to_look_for() {
        assert!(cases_command(&["orders.csv".to_owned()]).is_err());
    }

    #[test]
    fn parses_durations_with_units() {
        assert_eq!(parse_duration("45s").unwrap(), 45_000_000);
        assert_eq!(parse_duration("30m").unwrap(), 1_800_000_000);
        assert_eq!(parse_duration("6h").unwrap(), 21_600_000_000);
        assert_eq!(parse_duration("2d").unwrap(), 172_800_000_000);
        assert_eq!(parse_duration("1.5h").unwrap(), 5_400_000_000);
    }

    #[test]
    fn rejects_a_duration_without_a_unit() {
        assert!(parse_duration("30").is_err());
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn rejects_an_unknown_duration_unit() {
        assert!(parse_duration("5 fortnights").is_err());
    }

    #[test]
    fn a_flag_answer_overrides_a_saved_one() {
        let saved = Policy::new().answer(Kind::DuplicateRows, "deduplicate");
        let flags = Policy::new().answer(Kind::DuplicateRows, "keep_as_rework");
        assert_eq!(
            merge_policies(saved, flags).get(&Kind::DuplicateRows),
            Some("keep_as_rework")
        );
    }

    #[test]
    fn saved_answers_survive_when_no_flag_overrides_them() {
        let saved = Policy::new()
            .answer(Kind::DuplicateRows, "deduplicate")
            .answer(Kind::BlankTimestamps, "missing_data");
        let merged = merge_policies(saved, Policy::new());
        assert_eq!(merged.get(&Kind::DuplicateRows), Some("deduplicate"));
        assert_eq!(merged.get(&Kind::BlankTimestamps), Some("missing_data"));
    }

    #[test]
    fn detects_whether_mapping_flags_were_given() {
        assert!(mapping_flags_absent(&None, &None, &None));
        assert!(!mapping_flags_absent(&Some("c".into()), &None, &None));
    }

    #[test]
    fn truncates_long_values_and_marks_them() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("a-very-long-value", 8), "a-very-…");
    }

    #[test]
    fn inspect_requires_a_file() {
        assert!(inspect_command(&[]).is_err());
    }

    #[test]
    fn inspection_and_format_catalog_have_json_modes() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/canonical/01-sequential.csv")
            .display()
            .to_string();
        assert_eq!(
            inspect_command(&[source, "--json".to_owned()]).unwrap(),
            ExitCode::SUCCESS
        );
        assert_eq!(
            formats_command(&["--json".to_owned()]).unwrap(),
            ExitCode::SUCCESS
        );
    }

    #[test]
    fn uses_singular_and_plural_correctly() {
        assert_eq!(plural(1, "time", "times"), "1 time");
        assert_eq!(plural(2, "time", "times"), "2 times");
        assert_eq!(plural(0, "time", "times"), "0 times");
        assert_eq!(plural(1, "activity", "activities"), "1 activity");
    }

    #[test]
    fn formats_durations_at_sensible_scales() {
        assert_eq!(duration(45), "45s");
        assert_eq!(duration(600), "10m");
        assert_eq!(duration(3600), "1.0h");
        assert_eq!(duration(86_400 * 3), "3.0d");
    }

    #[test]
    fn every_decision_kind_round_trips_through_its_flag() {
        for kind in [
            Kind::BlankTimestamps,
            Kind::DuplicateRows,
            Kind::AmbiguousDateOrder,
            Kind::MixedZoneAndNaive,
            Kind::NearDuplicateLabels,
        ] {
            let flag = kind_flag(&kind);
            assert_eq!(
                parse_kind(flag).unwrap(),
                kind,
                "flag {flag} does not round-trip"
            );
        }
    }

    #[test]
    fn rejects_an_unknown_decision_name() {
        assert!(parse_kind("nonsense").is_err());
    }

    #[test]
    fn rejects_a_partial_mapping() {
        let args = vec!["file.csv".to_owned(), "--case".to_owned(), "c".to_owned()];
        assert!(analyze_command(&args).is_err());
    }

    #[test]
    fn requires_a_file() {
        assert!(analyze_command(&[]).is_err());
    }

    #[test]
    fn rejects_unknown_options() {
        let args = vec!["file.csv".to_owned(), "--wat".to_owned()];
        assert!(analyze_command(&args).is_err());
    }

    #[test]
    fn rejects_a_malformed_answer() {
        let args = vec![
            "file.csv".to_owned(),
            "--answer".to_owned(),
            "no_equals_sign".to_owned(),
        ];
        assert!(analyze_command(&args).is_err());
    }
}
