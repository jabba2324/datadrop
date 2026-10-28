//! Entity-relationship diagrams of a schema: a layout (tables in columns by how they
//! refer to each other), and the diagram as Mermaid or Graphviz text.

use crate::schema::{SchemaModel, TableModel};

/// The tables in columns, left to right: those that refer to nothing first, then each
/// table one column right of the furthest one it refers to. Views go in a last column.
pub fn columns(m: &SchemaModel) -> Vec<Vec<&TableModel>> {
    let tables: Vec<&TableModel> = m.tables.iter().filter(|t| !t.is_view()).collect();
    let mut level: Vec<Option<usize>> = vec![None; tables.len()];
    fn depth(
        i: usize,
        m: &SchemaModel,
        tables: &[&TableModel],
        level: &mut Vec<Option<usize>>,
        seen: &mut Vec<usize>,
    ) -> usize {
        if let Some(l) = level[i] {
            return l;
        }
        if seen.contains(&i) {
            // A cycle: stop here.
            return 0;
        }
        seen.push(i);
        let mut l = 0;
        for fk in tables[i].foreign_keys() {
            let Some(r) = &fk.references else { continue };
            if r.schema != m.schema || r.table == tables[i].name {
                continue;
            }
            if let Some(j) = tables.iter().position(|t| t.name == r.table) {
                l = l.max(depth(j, m, tables, level, seen) + 1);
            }
        }
        seen.pop();
        level[i] = Some(l);
        l
    }
    for i in 0..tables.len() {
        depth(i, m, &tables, &mut level, &mut vec![]);
    }
    let n = level.iter().flatten().max().map_or(0, |l| l + 1);
    let mut cols: Vec<Vec<&TableModel>> = vec![vec![]; n];
    for (i, t) in tables.iter().enumerate() {
        cols[level[i].unwrap_or(0)].push(t);
    }
    let views: Vec<&TableModel> = m.tables.iter().filter(|t| t.is_view()).collect();
    if !views.is_empty() {
        cols.push(views);
    }
    cols
}

/// A word Mermaid accepts as a type: `numeric(10,2)` becomes `numeric(10_2)`.
fn mermaid_word(s: &str) -> String {
    let w: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '-' | '(' | ')' | '[' | ']') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if w.starts_with(|c: char| c.is_ascii_digit()) {
        format!("_{w}")
    } else {
        w
    }
}

/// An `erDiagram` for Mermaid (GitHub and many wikis draw it).
pub fn mermaid(m: &SchemaModel) -> String {
    let mut s = String::from("erDiagram\n");
    for t in &m.tables {
        let pk: &[String] = t.primary_key().map_or(&[], |k| &k.columns);
        let fk: Vec<&String> = t.foreign_keys().flat_map(|f| &f.columns).collect();
        s.push_str(&format!("    {} {{\n", mermaid_word(&t.name)));
        for c in &t.columns {
            let mut keys = vec![];
            if pk.contains(&c.name) {
                keys.push("PK");
            }
            if fk.contains(&&c.name) {
                keys.push("FK");
            }
            s.push_str(&format!(
                "        {} {}{}\n",
                mermaid_word(&c.data_type),
                mermaid_word(&c.name),
                if keys.is_empty() {
                    String::new()
                } else {
                    format!(" {}", keys.join(", "))
                }
            ));
        }
        s.push_str("    }\n");
    }
    for t in &m.tables {
        for fk in t.foreign_keys() {
            let Some(r) = &fk.references else { continue };
            // Optional on the referring side if its columns can be NULL.
            let optional = fk
                .columns
                .iter()
                .any(|c| t.column(c).is_some_and(|c| c.nullable));
            s.push_str(&format!(
                "    {} {}--o{{ {} : \"{}\"\n",
                mermaid_word(&r.table),
                if optional { "|o" } else { "||" },
                mermaid_word(&t.name),
                fk.columns.join(", ").replace('"', "'")
            ));
        }
    }
    s
}

fn dot_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The diagram for Graphviz (`dot -Tsvg`).
pub fn dot(m: &SchemaModel) -> String {
    let mut s = String::from(
        "digraph schema {\n    rankdir=RL;\n    node [shape=plaintext, fontname=\"Helvetica\"];\n",
    );
    for t in &m.tables {
        let pk: &[String] = t.primary_key().map_or(&[], |k| &k.columns);
        let mut rows = String::new();
        for c in &t.columns {
            rows.push_str(&format!(
                "<tr><td port=\"{}\" align=\"left\">{}{}</td><td align=\"left\"><font color=\"#666666\">{}</font></td></tr>",
                dot_escape(&c.name),
                if pk.contains(&c.name) { "🔑 " } else { "" },
                dot_escape(&c.name),
                dot_escape(&c.data_type)
            ));
        }
        s.push_str(&format!(
            "    \"{}\" [label=<<table border=\"0\" cellborder=\"1\" cellspacing=\"0\"><tr><td colspan=\"2\" bgcolor=\"#dde6f0\"><b>{}</b></td></tr>{rows}</table>>];\n",
            dot_escape(&t.name),
            dot_escape(&t.name)
        ));
    }
    for t in &m.tables {
        for fk in t.foreign_keys() {
            let Some(r) = &fk.references else { continue };
            let from = fk
                .columns
                .first()
                .map(|c| dot_escape(c))
                .unwrap_or_default();
            let to = r.columns.first().map(|c| dot_escape(c)).unwrap_or_default();
            s.push_str(&format!(
                "    \"{}\":\"{from}\" -> \"{}\":\"{to}\";\n",
                dot_escape(&t.name),
                dot_escape(&r.table)
            ));
        }
    }
    s.push_str("}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::TableKind;
    use crate::schema::{ColumnModel, Constraint, ConstraintKind, Reference};

    fn table(name: &str, refs: &[&str]) -> TableModel {
        let mut columns = vec![ColumnModel {
            name: "id".into(),
            data_type: "integer".into(),
            ..ColumnModel::default()
        }];
        let mut constraints = vec![Constraint {
            name: format!("{name}_pkey"),
            kind: ConstraintKind::PrimaryKey,
            columns: vec!["id".into()],
            definition: String::new(),
            references: None,
        }];
        for r in refs {
            let col = format!("{r}_id");
            columns.push(ColumnModel {
                name: col.clone(),
                data_type: "numeric(10,2)".into(),
                nullable: true,
                ..ColumnModel::default()
            });
            constraints.push(Constraint {
                name: format!("{name}_{r}_fk"),
                kind: ConstraintKind::ForeignKey,
                columns: vec![col],
                definition: String::new(),
                references: Some(Reference {
                    schema: "s".into(),
                    table: (*r).into(),
                    columns: vec!["id".into()],
                    on_update: String::new(),
                    on_delete: String::new(),
                }),
            });
        }
        TableModel {
            name: name.into(),
            kind: TableKind::Table,
            comment: None,
            columns,
            constraints,
            indexes: vec![],
            triggers: vec![],
            definition: None,
        }
    }

    fn model() -> SchemaModel {
        SchemaModel {
            schema: "s".into(),
            tables: vec![
                table("order_lines", &["orders", "products"]),
                table("orders", &["customers"]),
                table("customers", &[]),
                table("products", &[]),
            ],
            ..SchemaModel::default()
        }
    }

    #[test]
    fn referenced_tables_go_left() {
        let m = model();
        let cols: Vec<Vec<&str>> = columns(&m)
            .iter()
            .map(|c| c.iter().map(|t| t.name.as_str()).collect())
            .collect();
        assert_eq!(
            cols,
            [
                vec!["customers", "products"],
                vec!["orders"],
                vec!["order_lines"]
            ]
        );
    }

    #[test]
    fn mermaid_and_dot() {
        let m = model();
        let md = mermaid(&m);
        assert!(
            md.contains(
                "    orders {\n        integer id PK\n        numeric(10_2) customers_id FK\n"
            ),
            "{md}"
        );
        assert!(
            md.contains("    customers |o--o{ orders : \"customers_id\""),
            "{md}"
        );
        let d = dot(&m);
        assert!(
            d.contains("\"orders\":\"customers_id\" -> \"customers\":\"id\";"),
            "{d}"
        );
    }
}
