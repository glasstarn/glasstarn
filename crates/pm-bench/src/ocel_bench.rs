//! Object-centric scale benchmark on the actual product query path.

use crate::{Phase, format_ms, time};
use pm_core::OcelFilter;
use rusqlite::{Connection, Transaction, params};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const STEPS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// Long native object lifecycles with one relationship per event.
    Lifecycle,
    /// Five participating objects and four static links per process instance.
    Interaction,
}

impl Shape {
    fn name(self) -> &'static str {
        match self {
            Self::Lifecycle => "lifecycle",
            Self::Interaction => "interaction",
        }
    }

    fn objects_per_instance(self) -> usize {
        match self {
            Self::Lifecycle => 1,
            Self::Interaction => 5,
        }
    }

    fn event_object_relations_per_event(self) -> usize {
        match self {
            Self::Lifecycle => 1,
            Self::Interaction => 5,
        }
    }

    fn object_object_relations_per_instance(self) -> usize {
        match self {
            Self::Lifecycle => 0,
            Self::Interaction => 4,
        }
    }
}

pub(crate) struct Run {
    events: usize,
    objects: usize,
    event_object_relations: usize,
    object_object_relations: usize,
    event_types: usize,
    object_types: usize,
    variants: usize,
    sqlite_bytes: u64,
    shape: Shape,
    phases: Vec<Phase>,
}

struct Expected {
    events: usize,
    objects: usize,
    event_object_relations: usize,
    object_object_relations: usize,
}

pub(crate) fn run(target_events: usize, shape: Shape) -> Run {
    let target_events = target_events.max(1);
    let temporary = TemporaryDatabase::new(shape);
    let path = &temporary.path;
    let instances = target_events.div_ceil(STEPS);

    let (expected, generate) = time(|| generate_sqlite(path, target_events, shape));
    let mut phases = vec![generate("generate OCEL SQLite")];
    let sqlite_bytes = std::fs::metadata(path)
        .expect("read generated OCEL metadata")
        .len();

    let (analysis, analyze) = time(|| {
        pm_query::analyze_ocel(path, &OcelFilter::default())
            .expect("object-centric product-path analysis")
    });
    phases.push(analyze("analyze OCEL (product path)"));

    assert!(analysis.ready_for_analysis, "generated OCEL must be ready");
    assert_eq!(analysis.scope.events_total, expected.events);
    assert_eq!(analysis.scope.events_shown, expected.events);
    assert_eq!(analysis.scope.objects_total, expected.objects);
    assert_eq!(analysis.scope.objects_shown, expected.objects);
    assert_eq!(
        analysis.scope.event_object_relations_total,
        expected.event_object_relations
    );
    assert_eq!(
        analysis.scope.object_object_relations_total,
        expected.object_object_relations
    );
    assert!(!analysis.scope.narrowed);
    assert_eq!(analysis.quality.summary.errors, 0);
    assert_eq!(instances * shape.objects_per_instance(), expected.objects);

    let variants = analysis
        .variants
        .as_ref()
        .expect("safe generated OCEL has variants")
        .len();
    Run {
        events: expected.events,
        objects: expected.objects,
        event_object_relations: expected.event_object_relations,
        object_object_relations: expected.object_object_relations,
        event_types: analysis.catalog.event_types.len(),
        object_types: analysis.catalog.object_types.len(),
        variants,
        sqlite_bytes,
        shape,
        phases,
    }
}

struct TemporaryDatabase {
    path: PathBuf,
}

impl TemporaryDatabase {
    fn new(shape: Shape) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after Unix epoch")
            .as_nanos();
        Self {
            path: std::env::temp_dir().join(format!(
                "pm-ocel-bench-{}-{}-{unique}.sqlite",
                std::process::id(),
                shape.name()
            )),
        }
    }
}

impl Drop for TemporaryDatabase {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!(
                "warning: could not remove benchmark database '{}': {error}",
                self.path.display()
            );
        }
    }
}

fn generate_sqlite(path: &Path, events: usize, shape: Shape) -> Expected {
    let mut connection = Connection::open(path).expect("create OCEL benchmark database");
    connection
        .execute_batch(
            "PRAGMA journal_mode = OFF;\n\
             PRAGMA synchronous = OFF;\n\
             PRAGMA temp_store = MEMORY;\n\
             PRAGMA foreign_keys = ON;",
        )
        .expect("configure benchmark database");
    let transaction = connection.transaction().expect("start OCEL generation");
    create_schema(&transaction, shape);
    let instances = events.div_ceil(STEPS);
    insert_objects(&transaction, instances, shape);
    insert_events(&transaction, events);
    insert_relationships(&transaction, events, shape);
    transaction.commit().expect("commit OCEL generation");
    let foreign_key_violations: u64 = connection
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .expect("validate generated OCEL foreign keys");
    assert_eq!(foreign_key_violations, 0, "generated OCEL foreign keys");
    drop(connection);

    Expected {
        events,
        objects: instances * shape.objects_per_instance(),
        event_object_relations: events * shape.event_object_relations_per_event(),
        object_object_relations: instances * shape.object_object_relations_per_instance(),
    }
}

fn create_schema(transaction: &Transaction<'_>, shape: Shape) {
    transaction
        .execute_batch(
            "CREATE TABLE event_corr_type (ocel_type TEXT PRIMARY KEY, ocel_type_corr TEXT UNIQUE NOT NULL);\n\
             CREATE TABLE object_corr_type (ocel_type TEXT PRIMARY KEY, ocel_type_corr TEXT UNIQUE NOT NULL);\n\
             CREATE TABLE event (ocel_id TEXT PRIMARY KEY, ocel_type TEXT NOT NULL REFERENCES event_corr_type (ocel_type));\n\
             CREATE TABLE object (ocel_id TEXT PRIMARY KEY, ocel_type TEXT NOT NULL REFERENCES object_corr_type (ocel_type));\n\
             CREATE TABLE event_object (ocel_event_id TEXT NOT NULL REFERENCES event (ocel_id), ocel_object_id TEXT NOT NULL REFERENCES object (ocel_id), ocel_qualifier TEXT NOT NULL, PRIMARY KEY (ocel_event_id, ocel_object_id, ocel_qualifier));\n\
             CREATE TABLE object_object (ocel_source_id TEXT NOT NULL REFERENCES object (ocel_id), ocel_target_id TEXT NOT NULL REFERENCES object (ocel_id), ocel_qualifier TEXT NOT NULL, PRIMARY KEY (ocel_source_id, ocel_target_id, ocel_qualifier));",
        )
        .expect("create OCEL core tables");
    for step in 0..STEPS {
        let activity = format!("step {step:02}");
        let mapped = format!("Step{step:02}");
        transaction
            .execute(
                "INSERT INTO event_corr_type VALUES (?1, ?2)",
                params![activity, mapped],
            )
            .expect("declare event type");
        let attribute = if step == 0 { ", amount INTEGER" } else { "" };
        transaction
            .execute_batch(&format!(
                "CREATE TABLE event_Step{step:02} (ocel_id TEXT PRIMARY KEY REFERENCES event (ocel_id), ocel_time TIMESTAMP NOT NULL{attribute});"
            ))
            .expect("create event type table");
    }
    let object_types: &[(&str, &str)] = match shape {
        Shape::Lifecycle => &[("order", "Order")],
        Shape::Interaction => &[
            ("order", "Order"),
            ("item", "Item"),
            ("employee", "Employee"),
        ],
    };
    for (name, mapped) in object_types {
        transaction
            .execute(
                "INSERT INTO object_corr_type VALUES (?1, ?2)",
                params![name, mapped],
            )
            .expect("declare object type");
        let attribute = if *name == "order" {
            ", status TEXT"
        } else {
            ""
        };
        transaction
            .execute_batch(&format!(
                "CREATE TABLE object_{mapped} (ocel_id TEXT NOT NULL REFERENCES object (ocel_id), ocel_time TIMESTAMP NOT NULL, ocel_changed_field TEXT{attribute});"
            ))
            .expect("create object type table");
    }
}

fn insert_objects(transaction: &Transaction<'_>, instances: usize, shape: Shape) {
    let mut object = transaction
        .prepare("INSERT INTO object VALUES (?1, ?2)")
        .expect("prepare object insert");
    let mut order = transaction
        .prepare("INSERT INTO object_Order VALUES (?1, '2024-01-01 00:00:00', NULL, 'open')")
        .expect("prepare order insert");
    let mut item = match shape {
        Shape::Lifecycle => None,
        Shape::Interaction => Some(
            transaction
                .prepare("INSERT INTO object_Item VALUES (?1, '2024-01-01 00:00:00', NULL)")
                .expect("prepare item insert"),
        ),
    };
    let mut employee = match shape {
        Shape::Lifecycle => None,
        Shape::Interaction => Some(
            transaction
                .prepare("INSERT INTO object_Employee VALUES (?1, '2024-01-01 00:00:00', NULL)")
                .expect("prepare employee insert"),
        ),
    };
    for instance in 0..instances {
        let order_id = format!("order-{instance:010}");
        object
            .execute(params![order_id, "order"])
            .expect("insert order object");
        order.execute([&order_id]).expect("insert typed order");
        if shape == Shape::Interaction {
            for position in 0..3 {
                let item_id = format!("item-{instance:010}-{position}");
                object
                    .execute(params![item_id, "item"])
                    .expect("insert item object");
                item.as_mut()
                    .expect("interaction item statement")
                    .execute([&item_id])
                    .expect("insert typed item");
            }
            let employee_id = format!("employee-{instance:010}");
            object
                .execute(params![employee_id, "employee"])
                .expect("insert employee object");
            employee
                .as_mut()
                .expect("interaction employee statement")
                .execute([&employee_id])
                .expect("insert typed employee");
        }
    }
}

fn insert_events(transaction: &Transaction<'_>, events: usize) {
    let mut event = transaction
        .prepare("INSERT INTO event VALUES (?1, ?2)")
        .expect("prepare event insert");
    let mut statements = (0..STEPS)
        .map(|step| {
            let attribute = if step == 0 { ", ?3" } else { "" };
            transaction
                .prepare(&format!(
                    "INSERT INTO event_Step{step:02} VALUES (?1, datetime(?2, 'unixepoch'){attribute})"
                ))
                .expect("prepare typed event insert")
        })
        .collect::<Vec<_>>();
    for index in 0..events {
        let step = index % STEPS;
        let event_id = format!("event-{index:010}");
        let activity = format!("step {step:02}");
        event
            .execute(params![event_id, activity])
            .expect("insert event");
        let timestamp = 1_704_067_200_i64 + index as i64;
        if step == 0 {
            statements[step]
                .execute(params![event_id, timestamp, index as i64])
                .expect("insert attributed typed event");
        } else {
            statements[step]
                .execute(params![event_id, timestamp])
                .expect("insert typed event");
        }
    }
}

fn insert_relationships(transaction: &Transaction<'_>, events: usize, shape: Shape) {
    let mut event_object = transaction
        .prepare("INSERT INTO event_object VALUES (?1, ?2, ?3)")
        .expect("prepare event-object insert");
    for index in 0..events {
        let instance = index / STEPS;
        let event_id = format!("event-{index:010}");
        let order_id = format!("order-{instance:010}");
        event_object
            .execute(params![event_id, order_id, "order"])
            .expect("relate event to order");
        if shape == Shape::Interaction {
            for position in 0..3 {
                let item_id = format!("item-{instance:010}-{position}");
                event_object
                    .execute(params![event_id, item_id, "item"])
                    .expect("relate event to item");
            }
            let employee_id = format!("employee-{instance:010}");
            event_object
                .execute(params![event_id, employee_id, "performer"])
                .expect("relate event to employee");
        }
    }
    if shape == Shape::Interaction {
        let instances = events.div_ceil(STEPS);
        let mut object_object = transaction
            .prepare("INSERT INTO object_object VALUES (?1, ?2, ?3)")
            .expect("prepare object-object insert");
        for instance in 0..instances {
            let order_id = format!("order-{instance:010}");
            for position in 0..3 {
                let item_id = format!("item-{instance:010}-{position}");
                object_object
                    .execute(params![order_id, item_id, "contains"])
                    .expect("relate order to item");
            }
            let employee_id = format!("employee-{instance:010}");
            object_object
                .execute(params![employee_id, order_id, "handles"])
                .expect("relate employee to order");
        }
    }
}

pub(crate) fn json(run: &Run) -> Value {
    let analyze = analysis_phase(run);
    let relationships = run.event_object_relations + run.object_object_relations;
    json!({
        "kind": "object_centric",
        "source_format": "ocel_2_sqlite",
        "shape": run.shape.name(),
        "events": run.events,
        "objects": run.objects,
        "event_object_relations": run.event_object_relations,
        "object_object_relations": run.object_object_relations,
        "event_types": run.event_types,
        "object_types": run.object_types,
        "variants": run.variants,
        "sqlite_bytes": run.sqlite_bytes,
        "analysis_millis": analyze.millis,
        "peak_rust_heap_mb": analyze.peak_mb,
        "per_million_events_millis": analyze.millis / run.events as f64 * 1_000_000.0,
        "per_million_relationships_millis": analyze.millis / relationships as f64 * 1_000_000.0,
        "memory_scope": "Rust allocator only; SQLite and other native allocations are excluded",
        "phases": run.phases.iter().map(|phase| json!({
            "phase": phase.name,
            "millis": phase.millis,
            "peak_mb": phase.peak_mb,
        })).collect::<Vec<_>>(),
    })
}

pub(crate) fn report(run: &Run) {
    println!(
        "\nOCEL {} · {} events · {} objects · {} E2O · {} O2O · {:.1} MB SQLite",
        run.shape.name(),
        run.events,
        run.objects,
        run.event_object_relations,
        run.object_object_relations,
        run.sqlite_bytes as f64 / 1_048_576.0
    );
    println!("{:<30} {:>12} {:>12}", "phase", "time", "peak heap");
    println!("{}", "-".repeat(56));
    for phase in &run.phases {
        println!(
            "{:<30} {:>12} {:>10.0} MB",
            phase.name,
            format_ms(phase.millis),
            phase.peak_mb
        );
    }
    let analyze = analysis_phase(run);
    println!("{}", "-".repeat(56));
    println!(
        "{:<30} {:>12}",
        "per million events",
        format_ms(analyze.millis / run.events as f64 * 1_000_000.0)
    );
    println!(
        "{:<30} {:>12}",
        "per million relationships",
        format_ms(
            analyze.millis / (run.event_object_relations + run.object_object_relations) as f64
                * 1_000_000.0
        )
    );
}

fn analysis_phase(run: &Run) -> &Phase {
    run.phases
        .iter()
        .find(|phase| phase.name == "analyze OCEL (product path)")
        .expect("analysis phase")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_shape_runs_the_real_product_path_and_reconciles() {
        let run = run(23, Shape::Lifecycle);
        assert_eq!(run.events, 23);
        assert_eq!(run.objects, 3);
        assert_eq!(run.event_object_relations, 23);
        assert_eq!(run.object_object_relations, 0);
        assert_eq!(run.event_types, 10);
        assert_eq!(run.object_types, 1);
    }

    #[test]
    fn interaction_shape_multiplies_relationship_pressure_without_event_copies() {
        let run = run(23, Shape::Interaction);
        assert_eq!(run.events, 23);
        assert_eq!(run.objects, 15);
        assert_eq!(run.event_object_relations, 115);
        assert_eq!(run.object_object_relations, 12);
        assert_eq!(run.event_types, 10);
        assert_eq!(run.object_types, 3);
        assert_eq!(json(&run)["events"], 23);
    }
}
