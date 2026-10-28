//! Query plans (EXPLAIN), turned from each database's own format into one tree.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{CoreError, Result};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlanNode {
    /// What the step does: "Seq Scan on orders", "Hash Join", ...
    pub label: String,
    /// Conditions, keys and the like: ("Filter", "(total > 100)").
    pub details: Vec<(String, String)>,
    pub estimated_rows: Option<f64>,
    /// The planner's cost estimate, in its own units.
    pub estimated_cost: Option<f64>,
    /// With ANALYZE: rows produced (per loop), loops, and the time spent in this step
    /// and the ones under it, in milliseconds, over all loops.
    pub actual_rows: Option<f64>,
    pub loops: Option<f64>,
    pub actual_ms: Option<f64>,
    pub children: Vec<PlanNode>,
}

impl PlanNode {
    /// Time spent in this step itself, not counting the ones under it.
    pub fn own_ms(&self) -> Option<f64> {
        let total = self.actual_ms?;
        let below: f64 = self.children.iter().filter_map(|c| c.actual_ms).sum();
        Some((total - below).max(0.0))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// Usually one; SQLite can have several top-level steps.
    pub roots: Vec<PlanNode>,
    /// The query really ran (EXPLAIN ANALYZE), so actual figures are filled in.
    pub analyzed: bool,
    pub planning_ms: Option<f64>,
    pub execution_ms: Option<f64>,
    /// What the database returned, unparsed.
    pub raw: String,
}

fn bad(what: &str) -> CoreError {
    CoreError::Database(format!("couldn't read the query plan: {what}"))
}

/// PostgreSQL's `EXPLAIN (FORMAT JSON)`.
pub fn parse_postgres(json: &str, analyzed: bool) -> Result<Plan> {
    let v: Value = serde_json::from_str(json).map_err(|e| bad(&e.to_string()))?;
    let top = v.get(0).ok_or_else(|| bad("empty"))?;
    let root = top.get("Plan").ok_or_else(|| bad("no Plan"))?;
    Ok(Plan {
        roots: vec![pg_node(root)],
        analyzed,
        planning_ms: top.get("Planning Time").and_then(Value::as_f64),
        execution_ms: top.get("Execution Time").and_then(Value::as_f64),
        raw: json.to_owned(),
    })
}

fn pg_node(n: &Value) -> PlanNode {
    let s = |k: &str| n.get(k).and_then(Value::as_str);
    let f = |k: &str| n.get(k).and_then(Value::as_f64);
    let mut label = s("Node Type").unwrap_or("?").to_owned();
    if let Some(strategy) = s("Strategy").filter(|st| *st != "Plain") {
        label = format!("{label} ({strategy})");
    }
    if let Some(join) = s("Join Type").filter(|j| *j != "Inner") {
        label = format!("{label} ({join})");
    }
    if let Some(index) = s("Index Name") {
        label.push_str(&format!(" using {index}"));
    }
    if let Some(rel) = s("Relation Name") {
        label.push_str(&format!(" on {rel}"));
        if let Some(alias) = s("Alias").filter(|a| *a != rel) {
            label.push_str(&format!(" {alias}"));
        }
    } else if let Some(cte) = s("CTE Name") {
        label.push_str(&format!(" on {cte}"));
    } else if let Some(func) = s("Function Name") {
        label.push_str(&format!(" on {func}"));
    }
    let mut details = vec![];
    for key in [
        "Index Cond",
        "Recheck Cond",
        "Filter",
        "Join Filter",
        "Hash Cond",
        "Merge Cond",
        "Rows Removed by Filter",
        "Rows Removed by Join Filter",
        "Sort Method",
        "Heap Fetches",
        "Subplan Name",
    ] {
        match n.get(key) {
            Some(Value::String(v)) => details.push((key.to_owned(), v.clone())),
            Some(Value::Number(v)) => details.push((key.to_owned(), v.to_string())),
            _ => {}
        }
    }
    for key in ["Sort Key", "Group Key", "Presorted Key"] {
        if let Some(Value::Array(items)) = n.get(key) {
            let joined: Vec<_> = items.iter().filter_map(Value::as_str).collect();
            details.push((key.to_owned(), joined.join(", ")));
        }
    }
    let loops = f("Actual Loops");
    PlanNode {
        label,
        details,
        estimated_rows: f("Plan Rows"),
        estimated_cost: f("Total Cost"),
        actual_rows: f("Actual Rows"),
        loops,
        actual_ms: f("Actual Total Time").map(|t| t * loops.unwrap_or(1.0)),
        children: n
            .get("Plans")
            .and_then(Value::as_array)
            .map(|c| c.iter().map(pg_node).collect())
            .unwrap_or_default(),
    }
}

/// MySQL's `EXPLAIN FORMAT=TREE` and `EXPLAIN ANALYZE`: one step per line, each
/// starting with `-> `, indented four spaces per level.
pub fn parse_mysql_tree(text: &str, analyzed: bool) -> Result<Plan> {
    let mut stack: Vec<(usize, PlanNode)> = vec![];
    let mut roots = vec![];
    for line in text.lines() {
        let trimmed = line.trim_start();
        let Some(body) = trimmed.strip_prefix("-> ") else {
            continue;
        };
        let depth = line.len() - trimmed.len();
        let node = mysql_node(body);
        // Close the steps at this depth or deeper: they're complete.
        while stack.last().is_some_and(|(d, _)| *d >= depth) {
            let (_, done) = stack.pop().expect("checked");
            match stack.last_mut() {
                Some((_, parent)) => parent.children.push(done),
                None => roots.push(done),
            }
        }
        stack.push((depth, node));
    }
    while let Some((_, done)) = stack.pop() {
        match stack.last_mut() {
            Some((_, parent)) => parent.children.push(done),
            None => roots.push(done),
        }
    }
    if roots.is_empty() {
        return Err(bad("no steps"));
    }
    let execution_ms = if analyzed {
        roots.first().and_then(|r| r.actual_ms)
    } else {
        None
    };
    Ok(Plan {
        roots,
        analyzed,
        planning_ms: None,
        execution_ms,
        raw: text.to_owned(),
    })
}

fn mysql_node(body: &str) -> PlanNode {
    // "Filter: (t.n > 5)  (cost=0.35 rows=1) (actual time=0.03..0.04 rows=2 loops=1)"
    let label_end = body
        .find("  (cost=")
        .or_else(|| body.find(" (cost="))
        .or_else(|| body.find(" (actual time="))
        .unwrap_or(body.len());
    let label = body[..label_end].trim().to_owned();
    let num_after = |key: &str, from: &str| -> Option<f64> {
        let i = from.find(key)? + key.len();
        let rest = &from[i..];
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-')))
            .unwrap_or(rest.len());
        rest[..end].parse().ok()
    };
    let est = body.find("(cost=").map(|i| &body[i..]).unwrap_or("");
    let est = &est[..est.find(')').map_or(est.len(), |e| e + 1)];
    let act = body.find("(actual time=").map(|i| &body[i..]).unwrap_or("");
    let loops = num_after("loops=", act);
    // "actual time=first..last": the time to the last row, per loop.
    let last_ms = act.find("..").and_then(|i| num_after("..", &act[i..]));
    // A label like "Filter: (cond)" keeps the condition as a detail too.
    let details = match label.split_once(": ") {
        Some((kind, rest)) if !rest.is_empty() && kind.len() < 30 => {
            vec![(kind.to_owned(), rest.to_owned())]
        }
        _ => vec![],
    };
    PlanNode {
        label,
        details,
        estimated_rows: num_after("rows=", est),
        estimated_cost: num_after("cost=", est),
        actual_rows: num_after("rows=", act),
        loops,
        actual_ms: last_ms.map(|t| t * loops.unwrap_or(1.0)),
        children: vec![],
    }
}

/// SQLite's `EXPLAIN QUERY PLAN`: rows of (id, parent, unused, detail).
pub fn parse_sqlite(rows: &[(i64, i64, String)]) -> Result<Plan> {
    fn build(parent: i64, rows: &[(i64, i64, String)]) -> Vec<PlanNode> {
        rows.iter()
            .filter(|(_, p, _)| *p == parent)
            .map(|(id, _, detail)| PlanNode {
                label: detail.clone(),
                children: build(*id, rows),
                ..PlanNode::default()
            })
            .collect()
    }
    let roots = build(0, rows);
    if roots.is_empty() {
        return Err(bad("no steps"));
    }
    let raw = rows
        .iter()
        .map(|(id, p, d)| format!("{id}|{p}|{d}"))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Plan {
        roots,
        raw,
        ..Plan::default()
    })
}

/// SQL Server's `SHOWPLAN_ALL` (or, after a run, `STATISTICS PROFILE`) result: a row a
/// step, with `NodeId` and `Parent` making the tree.
pub fn parse_sqlserver(r: &crate::db::StatementResult, analyzed: bool) -> Result<Plan> {
    let col = |name: &str| r.columns.iter().position(|c| c.name == name);
    let (Some(id), Some(parent)) = (col("NodeId"), col("Parent")) else {
        return Err(bad("no NodeId and Parent columns"));
    };
    let get = |row: &Vec<Option<String>>, c: Option<usize>| -> Option<String> {
        c.and_then(|c| row.get(c).cloned().flatten())
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
    };
    let num = |row: &Vec<Option<String>>, c: Option<usize>| -> Option<f64> {
        get(row, c).and_then(|s| s.parse().ok())
    };
    struct Step {
        id: i64,
        parent: i64,
        node: PlanNode,
    }
    let mut steps = vec![];
    for row in &r.rows {
        let Some(nid) = get(row, Some(id)).and_then(|s| s.parse().ok()) else {
            continue;
        };
        let pid = get(row, Some(parent))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let physical = get(row, col("PhysicalOp"));
        let logical = get(row, col("LogicalOp"));
        let label = match (&physical, &logical) {
            (Some(p), Some(l)) if p != l => format!("{p} ({l})"),
            (Some(p), _) => p.clone(),
            (None, _) => get(row, col("StmtText")).unwrap_or_default(),
        };
        let mut details = vec![];
        for (key, column) in [("Argument", "Argument"), ("Output", "OutputList")] {
            if let Some(v) = get(row, col(column)) {
                details.push((key.to_owned(), v));
            }
        }
        steps.push(Step {
            id: nid,
            parent: pid,
            node: PlanNode {
                label,
                details,
                estimated_rows: num(row, col("EstimateRows")),
                estimated_cost: num(row, col("TotalSubtreeCost")),
                actual_rows: num(row, col("Rows")),
                loops: num(row, col("Executes")),
                ..PlanNode::default()
            },
        });
    }
    fn build(parent: i64, steps: &[Step]) -> Vec<PlanNode> {
        steps
            .iter()
            .filter(|s| s.parent == parent && s.id != parent)
            .map(|s| {
                let mut n = s.node.clone();
                n.children = build(s.id, steps);
                n
            })
            .collect()
    }
    // The statement itself is the node no other step is the parent of... the top
    // step's parent is 0 (and the statement row's NodeId is 1, or the plan's root).
    let mut roots = build(0, &steps);
    if roots.is_empty()
        && let Some(min) = steps.iter().map(|s| s.parent).min()
    {
        roots = build(min, &steps);
    }
    if roots.is_empty() {
        return Err(bad("no steps"));
    }
    let raw = r
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|v| v.clone().unwrap_or_default())
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Plan {
        roots,
        analyzed,
        raw,
        ..Plan::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_json() {
        let json = r#"[{"Plan": {"Node Type": "Hash Join", "Join Type": "Left", "Total Cost": 12.5,
            "Plan Rows": 10, "Actual Rows": 8, "Actual Loops": 1, "Actual Total Time": 2.0,
            "Hash Cond": "(o.customer_id = c.id)",
            "Plans": [
              {"Node Type": "Seq Scan", "Relation Name": "orders", "Alias": "o", "Total Cost": 4.0,
               "Plan Rows": 100, "Actual Rows": 100, "Actual Loops": 1, "Actual Total Time": 0.5,
               "Filter": "(total > 10)", "Rows Removed by Filter": 3},
              {"Node Type": "Hash", "Total Cost": 3.0, "Plan Rows": 5, "Actual Rows": 5,
               "Actual Loops": 1, "Actual Total Time": 0.25,
               "Plans": [{"Node Type": "Index Scan", "Index Name": "customers_pkey",
                 "Relation Name": "customers", "Alias": "customers", "Actual Loops": 2,
                 "Actual Total Time": 0.1, "Sort Key": ["id", "name"]}]}
            ]},
            "Planning Time": 0.3, "Execution Time": 2.4}]"#;
        let p = parse_postgres(json, true).unwrap();
        let root = &p.roots[0];
        assert_eq!(root.label, "Hash Join (Left)");
        assert_eq!(
            root.details,
            [("Hash Cond".into(), "(o.customer_id = c.id)".into())]
        );
        assert_eq!(root.children[0].label, "Seq Scan on orders o");
        assert!(
            root.children[0]
                .details
                .contains(&("Rows Removed by Filter".into(), "3".into()))
        );
        let index = &root.children[1].children[0];
        assert_eq!(index.label, "Index Scan using customers_pkey on customers");
        assert_eq!(index.actual_ms, Some(0.2), "per-loop time times loops");
        assert_eq!(index.details, [("Sort Key".into(), "id, name".into())]);
        assert_eq!(root.own_ms(), Some(1.25));
        assert_eq!(p.execution_ms, Some(2.4));
        assert!(parse_postgres("[]", false).is_err());
    }

    #[test]
    fn mysql_tree() {
        let text = "-> Nested loop inner join  (cost=1.2 rows=3) (actual time=0.05..0.4 rows=2 loops=1)\n    \
                    -> Filter: (o.total > 100)  (cost=0.55 rows=1) (actual time=0.02..0.03 rows=2 loops=1)\n        \
                    -> Table scan on o  (cost=0.55 rows=3) (actual time=0.01..0.02 rows=3 loops=1)\n    \
                    -> Single-row index lookup on c using PRIMARY (id=o.customer_id)  (cost=0.3 rows=1) (actual time=0.1..0.15 rows=1 loops=2)\n";
        let p = parse_mysql_tree(text, true).unwrap();
        assert_eq!(p.roots.len(), 1);
        let root = &p.roots[0];
        assert_eq!(root.label, "Nested loop inner join");
        assert_eq!(root.estimated_cost, Some(1.2));
        assert_eq!(root.actual_ms, Some(0.4));
        assert_eq!(root.children.len(), 2);
        let filter = &root.children[0];
        assert_eq!(
            filter.details,
            [("Filter".into(), "(o.total > 100)".into())]
        );
        assert_eq!(filter.children[0].label, "Table scan on o");
        assert_eq!(root.children[1].actual_ms, Some(0.3));
        assert_eq!(root.children[1].estimated_rows, Some(1.0));
        assert_eq!(p.execution_ms, Some(0.4));
        // Without ANALYZE: estimates only.
        let p = parse_mysql_tree("-> Table scan on t  (cost=0.35 rows=1)\n", false).unwrap();
        assert_eq!(p.roots[0].label, "Table scan on t");
        assert_eq!(p.roots[0].actual_ms, None);
    }

    #[test]
    fn sqlite_rows() {
        let rows = vec![
            (2, 0, "SCAN o".to_owned()),
            (
                5,
                0,
                "SEARCH c USING INTEGER PRIMARY KEY (rowid=?)".to_owned(),
            ),
            (9, 0, "USE TEMP B-TREE FOR ORDER BY".to_owned()),
            (12, 5, "inner".to_owned()),
        ];
        let p = parse_sqlite(&rows).unwrap();
        assert_eq!(p.roots.len(), 3);
        assert_eq!(p.roots[1].children[0].label, "inner");
        assert!(parse_sqlite(&[]).is_err());
    }
}
