//! `query plan`: the plan Apollo Router follows to run an operation against a
//! supergraph, from the query planner in `apollo-federation`, the crate the
//! Router itself plans with, configured as the Router's defaults configure it.
//!
//! The plan prints as the planner's own text, as the Router returns it to a
//! client that asks with `Apollo-Expose-Query-Plan`, or as the
//! JSON the Router returns beside that text, which `json` builds from the
//! planner's types.
//!
//! The documents are read as every other command reads them (`Input`), so a
//! syntax error, a repeated name, and a schema that is not a supergraph are
//! reported as they are everywhere else. Only then are they handed to the
//! planner, whose own parser and validation decide the rest.

use crate::{
    error::Error,
    input::{Input, Origin},
    query_target,
    supergraph::{string_argument, Supergraph},
    util, Output,
};
use apollo_compiler::{
    validation::{DiagnosticList, Valid},
    ExecutableDocument, Name,
};
use apollo_federation::query_plan::{
    query_planner::{
        QueryPlanIncrementalDeliveryConfig, QueryPlanOptions, QueryPlanner, QueryPlannerConfig,
    },
    ConditionNode, DeferNode, FetchDataPathElement, FetchDataRewrite, FetchNode, FlattenNode,
    PlanNode, QueryPathElement, QueryPlan, TopLevelPlanNode,
};
use graphql_parser::schema::{Document, TypeDefinition};
use serde_json::{json, Map, Value};
use std::{borrow::Cow, collections::BTreeSet};

/// How `query plan` prints the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// The planner's own text: `QueryPlan { Sequence { Fetch(service: ...) } }`.
    Text,
    /// The JSON the Router returns: `{"kind": "QueryPlan", "node": ...}`.
    Json,
}

/// What `query plan` is asked to plan, beyond the two documents.
#[derive(Debug, Clone, Copy)]
pub struct Options<'a> {
    /// The operation to plan, needed when the query has more than one.
    pub operation: Option<&'a str>,
    /// The progressive `@override` labels to plan as on. The Router turns a
    /// `percent(N)` label on for that share of requests, and others as a
    /// plugin decides, so a plan with none on is only one of the plans.
    pub override_labels: &'a [String],
    pub format: Format,
}

/// Plans an operation of the query `read_query` reads against the supergraph
/// `schema`, as `options` asks, and prints the plan.
///
/// The schema is what the query is resolved against, so a blank one is
/// `Error::BlankSchema`, as for the other query commands, and one that is not
/// a supergraph is `Error::NotASupergraph`. The query is read only once the
/// planner accepts the schema, since reading it may wait on stdin. A blank
/// query passes through as empty.
///
/// Several operations need `options.operation` to name one
/// (`Error::OperationNameNeeded`), and a name the query does not define is
/// `Error::UnknownOperation`. What the planner rejects is
/// `Error::MalformedSupergraph` for the schema, `Error::InvalidQuery` for a
/// query that does not validate against the supergraph's API schema, and
/// `Error::NoPlan` for one it cannot plan. An override label no `@override`
/// in the supergraph has changes nothing, and gets a note.
pub fn process(
    schema: &Input,
    read_query: impl FnOnce() -> Result<Input, Error>,
    options: Options<'_>,
    warnings: &mut Vec<String>,
) -> Result<Output, Error> {
    let (planner, labels) = planner(schema, warnings)?;
    let notes: Vec<String> = options
        .override_labels
        .iter()
        .filter(|label| !labels.contains(label.as_str()))
        .map(|label| {
            format!(
                "no `@override` in {} has the label `{label}`, so it changes nothing",
                schema.origin()
            )
        })
        .collect();
    let query = &read_query()?;

    let before = warnings.len();
    let Some(parsed) = query.parse_query(warnings)? else {
        return Ok(Output {
            document: String::new(),
            notes,
        });
    };
    let text = as_read(query, || parsed.to_string(), warnings.len() > before);
    let document = ExecutableDocument::parse_and_validate(
        planner.api_schema().schema(),
        text.as_ref(),
        file_name(query.origin()),
    )
    .map_err(|invalid| Error::InvalidQuery {
        origin: query.origin().clone(),
        message: diagnostics(&invalid.errors),
    })?;

    let operation_name = pick_operation(&document, options.operation, query.origin())?;
    let plan_options = QueryPlanOptions {
        override_conditions: options.override_labels.to_vec(),
        ..QueryPlanOptions::default()
    };
    let plan = planner
        .build_query_plan(&document, operation_name, plan_options)
        .map_err(|e| Error::NoPlan {
            origin: query.origin().clone(),
            message: e.to_string(),
        })?;

    let document = match options.format {
        Format::Text => plan.to_string(),
        Format::Json => {
            serde_json::to_string_pretty(&json(&plan)).expect("a JSON value always serializes")
        }
    };
    Ok(Output { document, notes })
}

/// The planner for the supergraph `schema`, configured as the Router's
/// defaults configure it: subgraph operations reuse repeated selections as
/// named fragments, and `@defer` is planned rather than ignored. With it, the
/// labels of the supergraph's progressive `@override`s.
fn planner(
    schema: &Input,
    warnings: &mut Vec<String>,
) -> Result<(QueryPlanner, BTreeSet<String>), Error> {
    let before = warnings.len();
    let parsed = schema
        .parse_schema(warnings)?
        .ok_or_else(|| Error::BlankSchema {
            origin: schema.origin().clone(),
        })?;
    let Some(supergraph) = Supergraph::detect(&parsed) else {
        return Err(Error::NotASupergraph {
            origin: schema.origin().clone(),
        });
    };
    let labels = override_labels(&supergraph, &parsed);
    let text = as_read(schema, || parsed.to_string(), warnings.len() > before);

    let malformed = |e: &dyn std::fmt::Display| Error::MalformedSupergraph {
        origin: schema.origin().clone(),
        message: e.to_string(),
    };
    let supergraph =
        apollo_federation::Supergraph::new_with_router_specs(&text).map_err(|e| malformed(&e))?;
    let config = QueryPlannerConfig {
        generate_query_fragments: true,
        incremental_delivery: QueryPlanIncrementalDeliveryConfig { enable_defer: true },
        ..QueryPlannerConfig::default()
    };
    let planner = QueryPlanner::new(&supergraph, config).map_err(|e| malformed(&e))?;
    Ok((planner, labels))
}

/// Every `overrideLabel:` of a `@join__field` in the supergraph.
fn override_labels(supergraph: &Supergraph, doc: &Document<'_, String>) -> BTreeSet<String> {
    let mut labels = BTreeSet::new();
    for td in util::merged_type_definitions(doc).values() {
        let fields = match td {
            TypeDefinition::Object(t) => &t.fields,
            TypeDefinition::Interface(t) => &t.fields,
            _ => continue,
        };
        for field in fields {
            for join in supergraph.joins(&field.directives, "field") {
                if let Some(label) = string_argument(join, "overrideLabel") {
                    labels.insert(label.to_string());
                }
            }
        }
    }
    labels
}

/// The text to hand the planner: the input as written, so its positions are
/// the ones the user sees, unless `Input` dropped a repeated definition from
/// it, when it is `reprinted` without the repeats the warnings say are
/// ignored.
fn as_read<'i>(
    input: &'i Input,
    reprinted: impl FnOnce() -> String,
    dropped: bool,
) -> Cow<'i, str> {
    if dropped {
        Cow::Owned(reprinted())
    } else {
        Cow::Borrowed(input.text())
    }
}

/// The name apollo-compiler reports diagnostics against, which this tool
/// never prints, since `diagnostics` gives only their positions.
fn file_name(origin: &Origin) -> String {
    match origin {
        Origin::File(path) => path.display().to_string(),
        Origin::Stdin => "(stdin)".to_string(),
    }
}

/// apollo-compiler's diagnostics as one line, each as `line:column: message`,
/// joined with `; `, the way `Error::parse` joins graphql-parser's.
fn diagnostics(list: &DiagnosticList) -> String {
    list.iter()
        .map(|diagnostic| {
            let message = diagnostic.error.to_string();
            match diagnostic.line_column_range() {
                Some(range) => format!("{}:{}: {message}", range.start.line, range.start.column),
                None => message,
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// The operation to plan, as GraphQL's `GetOperation` picks it: the one
/// `name` names, or the only one when no name is given.
fn pick_operation(
    document: &Valid<ExecutableDocument>,
    name: Option<&str>,
    origin: &Origin,
) -> Result<Option<Name>, Error> {
    let named: Vec<&str> = document.operations.named.keys().map(Name::as_str).collect();
    match name {
        Some(name) => {
            let Some((found, _)) = document.operations.named.get_key_value(name) else {
                return Err(Error::UnknownOperation {
                    name: name.to_string(),
                    origin: origin.clone(),
                    suggestion: query_target::suggest(name, named.iter().copied()),
                    available: named.iter().map(|n| n.to_string()).collect(),
                });
            };
            Ok(Some(found.clone()))
        }
        None => {
            let count = named.len() + usize::from(document.operations.anonymous.is_some());
            if count > 1 {
                return Err(Error::OperationNameNeeded {
                    origin: origin.clone(),
                    available: named.iter().map(|n| n.to_string()).collect(),
                });
            }
            Ok(document.operations.named.keys().next().cloned())
        }
    }
}

/// The plan as the JSON Apollo Router returns beside its text, under
/// `extensions.apolloQueryPlan.object`: a `QueryPlan` whose nodes are each
/// tagged by `kind`, with camelCase fields. The Router adds two fields of its
/// own to each fetch, `schemaAwareHash` and `authorization`, which the planner
/// does not decide and which are left out.
pub fn json(plan: &QueryPlan) -> Value {
    let node = plan.node.as_ref().map_or(Value::Null, |node| match node {
        TopLevelPlanNode::Subscription(subscription) => json!({
            "kind": "Subscription",
            "primary": subscription_primary(&subscription.primary),
            "rest": optional_node(subscription.rest.as_deref()),
        }),
        TopLevelPlanNode::Fetch(fetch) => fetch_node(fetch),
        TopLevelPlanNode::Sequence(sequence) => nodes("Sequence", &sequence.nodes),
        TopLevelPlanNode::Parallel(parallel) => nodes("Parallel", &parallel.nodes),
        TopLevelPlanNode::Flatten(flatten) => flatten_node(flatten),
        TopLevelPlanNode::Defer(defer) => defer_node(defer),
        TopLevelPlanNode::Condition(condition) => condition_node(condition),
    });
    json!({ "kind": "QueryPlan", "node": node })
}

/// A node below the top level, which can be any kind but a subscription.
fn plan_node(node: &PlanNode) -> Value {
    match node {
        PlanNode::Fetch(fetch) => fetch_node(fetch),
        PlanNode::Sequence(sequence) => nodes("Sequence", &sequence.nodes),
        PlanNode::Parallel(parallel) => nodes("Parallel", &parallel.nodes),
        PlanNode::Flatten(flatten) => flatten_node(flatten),
        PlanNode::Defer(defer) => defer_node(defer),
        PlanNode::Condition(condition) => condition_node(condition),
    }
}

fn optional_node(node: Option<&PlanNode>) -> Value {
    node.map_or(Value::Null, plan_node)
}

fn nodes(kind: &str, nodes: &[PlanNode]) -> Value {
    json!({ "kind": kind, "nodes": nodes.iter().map(plan_node).collect::<Vec<_>>() })
}

fn flatten_node(flatten: &FlattenNode) -> Value {
    json!({
        "kind": "Flatten",
        "path": data_path(&flatten.path),
        "node": plan_node(&flatten.node),
    })
}

fn condition_node(condition: &ConditionNode) -> Value {
    json!({
        "kind": "Condition",
        "condition": condition.condition_variable.as_str(),
        "ifClause": optional_node(condition.if_clause.as_deref()),
        "elseClause": optional_node(condition.else_clause.as_deref()),
    })
}

/// A fetch from one subgraph. `requires`, the selection of the entities a
/// fetch sends as `$representations`, is there only when it has one.
fn fetch_node(fetch: &FetchNode) -> Value {
    let mut node = Map::new();
    node.insert("kind".into(), "Fetch".into());
    node.insert("serviceName".into(), fetch.subgraph_name.as_ref().into());
    if !fetch.requires.is_empty() {
        node.insert(
            "requires".into(),
            serde_json::to_value(&fetch.requires).expect("a selection serializes"),
        );
    }
    node.insert("variableUsages".into(), names(&fetch.variable_usages));
    node.insert(
        "operation".into(),
        fetch.operation_document.as_serialized().into(),
    );
    node.insert(
        "operationName".into(),
        fetch.operation_name.as_ref().map(Name::as_str).into(),
    );
    node.insert("operationKind".into(), fetch.operation_kind.name().into());
    node.insert("id".into(), fetch.id.map(|id| id.to_string()).into());
    node.insert("inputRewrites".into(), rewrites(&fetch.input_rewrites));
    node.insert("outputRewrites".into(), rewrites(&fetch.output_rewrites));
    node.insert("contextRewrites".into(), rewrites(&fetch.context_rewrites));
    Value::Object(node)
}

/// A subscription's first fetch, which has no `requires` or `id`, as it is
/// always the one fetch from the root.
fn subscription_primary(fetch: &FetchNode) -> Value {
    json!({
        "serviceName": fetch.subgraph_name.as_ref(),
        "variableUsages": names(&fetch.variable_usages),
        "operation": fetch.operation_document.as_serialized(),
        "operationName": fetch.operation_name.as_ref().map(Name::as_str),
        "operationKind": fetch.operation_kind.name(),
        "inputRewrites": rewrites(&fetch.input_rewrites),
        "outputRewrites": rewrites(&fetch.output_rewrites),
    })
}

fn defer_node(defer: &DeferNode) -> Value {
    let deferred: Vec<Value> = defer
        .deferred
        .iter()
        .map(|block| {
            let query_path: Vec<String> = block
                .query_path
                .iter()
                .map(|element| match element {
                    QueryPathElement::Field { response_key } => response_key.to_string(),
                    QueryPathElement::InlineFragment { type_condition } => {
                        format!("... on {type_condition}")
                    }
                })
                .collect();
            json!({
                "depends": block.depends.iter().map(|d| json!({ "id": d.id })).collect::<Vec<_>>(),
                "label": block.label,
                "queryPath": query_path,
                "subselection": block.sub_selection,
                "node": block.node.as_deref().map_or(Value::Null, plan_node),
            })
        })
        .collect();
    json!({
        "kind": "Defer",
        "primary": {
            "subselection": defer.primary.sub_selection,
            "node": defer.primary.node.as_deref().map_or(Value::Null, plan_node),
        },
        "deferred": deferred,
    })
}

fn names(names: &[Name]) -> Value {
    names.iter().map(Name::as_str).collect::<Vec<_>>().into()
}

/// Rewrites as the Router lists them, `null` when there are none.
fn rewrites<R: AsRef<FetchDataRewrite>>(rewrites: &[R]) -> Value {
    if rewrites.is_empty() {
        return Value::Null;
    }
    rewrites
        .iter()
        .map(|rewrite| match rewrite.as_ref() {
            FetchDataRewrite::ValueSetter(setter) => json!({
                "kind": "ValueSetter",
                "path": data_path(&setter.path),
                "setValueTo": serde_json::to_value(&setter.set_value_to)
                    .expect("a JSON value serializes"),
            }),
            FetchDataRewrite::KeyRenamer(renamer) => json!({
                "kind": "KeyRenamer",
                "path": data_path(&renamer.path),
                "renameKeyTo": renamer.rename_key_to.as_str(),
            }),
        })
        .collect::<Vec<_>>()
        .into()
}

/// A path into response data as the Router writes one: a key, `@` for every
/// element of a list, `... on Type` for objects of that `__typename`, and
/// `..` for the parent. A key or `@` that applies only to some types ends in
/// `|[A,B]`.
fn data_path(path: &[FetchDataPathElement]) -> Value {
    let conditioned = |base: &str, conditions: &Option<Vec<Name>>| match conditions {
        Some(types) => {
            let types: Vec<&str> = types.iter().map(Name::as_str).collect();
            format!("{base}|[{}]", types.join(","))
        }
        None => base.to_string(),
    };
    path.iter()
        .map(|element| match element {
            FetchDataPathElement::Key(name, conditions) => conditioned(name.as_str(), conditions),
            FetchDataPathElement::AnyIndex(conditions) => conditioned("@", conditions),
            FetchDataPathElement::TypenameEquals(type_name) => format!("... on {type_name}"),
            FetchDataPathElement::Parent => "..".to_string(),
        })
        .collect::<Vec<_>>()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    const SUPERGRAPH: &str = include_str!("../tests/fixtures/supergraph.graphql");
    const QUERY: &str = include_str!("../tests/fixtures/supergraph-query.graphql");

    fn options<'a>(
        operation: Option<&'a str>,
        labels: &'a [String],
        format: Format,
    ) -> Options<'a> {
        Options {
            operation,
            override_labels: labels,
            format,
        }
    }

    fn plan_with(query: &str, options: Options<'_>) -> Result<Output, Error> {
        let schema = Input::inline(SUPERGRAPH);
        process(
            &schema,
            || Ok(Input::inline(query)),
            options,
            &mut Vec::new(),
        )
    }

    /// Plans `query` against the fixture supergraph as text.
    fn plan(query: &str) -> String {
        plan_with(query, options(None, &[], Format::Text))
            .unwrap()
            .document
    }

    fn json_plan(query: &str) -> Value {
        let output = plan_with(query, options(None, &[], Format::Json)).unwrap();
        serde_json::from_str(&output.document).unwrap()
    }

    fn error(schema: &str, query: &str, operation: Option<&str>) -> String {
        let schema = Input::inline(schema);
        let options = options(operation, &[], Format::Text);
        match process(
            &schema,
            || Ok(Input::inline(query)),
            options,
            &mut Vec::new(),
        ) {
            Ok(output) => panic!("expected an error, got:\n{}", output.document),
            Err(error) => error.to_string(),
        }
    }

    /// The fixture's query fetches products first, then the reviews of each,
    /// sending the reviews subgraph the `weight` that `shipping` requires.
    #[test]
    fn plans_a_query_across_subgraphs() {
        assert_eq!(
            plan(QUERY),
            indoc! {r#"
                QueryPlan {
                  Sequence {
                    Fetch(service: "products") {
                      {
                        topProducts(first: 3) {
                          __typename
                          upc
                          name
                          weight
                        }
                      }
                    },
                    Flatten(path: "topProducts.@") {
                      Fetch(service: "reviews") {
                        {
                          ... on Product {
                            __typename
                            upc
                            weight
                          }
                        } =>
                        {
                          ... on Product {
                            shipping
                            reviews {
                              body
                              author {
                                username
                              }
                            }
                          }
                        }
                      },
                    },
                  },
                }"#}
        );
    }

    /// As with the Router's defaults, a selection a subgraph operation
    /// repeats is sent as a named fragment.
    #[test]
    fn subgraph_operations_reuse_selections_as_fragments() {
        let plan = plan(indoc! {"
            {
              a: topProducts(first: 1) { upc name weight price { amount currency } kind }
              b: topProducts(first: 2) { upc name weight price { amount currency } kind }
            }
        "});
        assert!(plan.contains("fragment b on Product {"), "{plan}");
        assert!(
            plan.contains("a: topProducts(first: 1) {\n        ...b"),
            "{plan}"
        );
    }

    /// As with the Router's defaults, `@defer` is planned: the deferred part
    /// is fetched after the primary one, in a block of its own.
    #[test]
    fn plans_defer() {
        let plan = plan("{ topProducts { name ... @defer { reviews { body } } } }");
        assert!(plan.contains("  Defer {\n    Primary {"), "{plan}");
        assert!(
            plan.contains(r#"Deferred(depends: [0], path: "topProducts") {"#),
            "{plan}"
        );
    }

    /// `--json` prints the plan as the Router's JSON: nodes tagged by `kind`,
    /// in camelCase, in the Router's field order.
    #[test]
    fn json_is_the_routers() {
        let plan = json_plan("{ me { username } }");
        assert_eq!(
            plan,
            json!({
                "kind": "QueryPlan",
                "node": {
                    "kind": "Fetch",
                    "serviceName": "accounts",
                    "variableUsages": [],
                    "operation": "{ me { username } }",
                    "operationName": null,
                    "operationKind": "query",
                    "id": null,
                    "inputRewrites": null,
                    "outputRewrites": null,
                    "contextRewrites": null,
                },
            })
        );
        let keys: Vec<&String> = plan["node"].as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "kind",
                "serviceName",
                "variableUsages",
                "operation",
                "operationName",
                "operationKind",
                "id",
                "inputRewrites",
                "outputRewrites",
                "contextRewrites",
            ]
        );
    }

    /// An entity fetch has its `requires`, and its `Flatten` the path into
    /// the response it merges into, `@` standing for each list element.
    #[test]
    fn json_keeps_requires_and_paths() {
        let plan = json_plan(QUERY);
        let flatten = &plan["node"]["nodes"][1];
        assert_eq!(flatten["kind"], "Flatten");
        assert_eq!(flatten["path"], json!(["topProducts", "@"]));
        assert_eq!(flatten["node"]["serviceName"], "reviews");
        assert_eq!(
            flatten["node"]["requires"],
            json!([{
                "kind": "InlineFragment",
                "typeCondition": "Product",
                "selections": [
                    { "kind": "Field", "name": "__typename" },
                    { "kind": "Field", "name": "upc" },
                    { "kind": "Field", "name": "weight" },
                ],
            }])
        );
    }

    /// The paths the Router writes: a key or `@` with the types it applies
    /// to, a `__typename` condition, and the parent.
    #[test]
    fn data_paths_are_written_as_the_router_writes_them() {
        let name = |n: &str| Name::new(n).unwrap();
        let path = [
            FetchDataPathElement::Key(name("a"), None),
            FetchDataPathElement::AnyIndex(Some(vec![name("A"), name("B")])),
            FetchDataPathElement::Key(name("b"), Some(vec![name("C")])),
            FetchDataPathElement::TypenameEquals(name("D")),
            FetchDataPathElement::Parent,
        ];
        assert_eq!(
            data_path(&path),
            json!(["a", "@|[A,B]", "b|[C]", "... on D", ".."])
        );
    }

    #[test]
    fn several_operations_need_a_name() {
        let query = "query A { me { id } } query B { topProducts { upc } }";
        assert_eq!(
            error(SUPERGRAPH, query, None),
            "query (stdin) has several operations; pass --operation with the one to plan; its operations are `A` and `B`"
        );
    }

    #[test]
    fn plans_the_operation_named() {
        let query = "query A { me { id } } query B { topProducts { upc } }";
        let output = plan_with(query, options(Some("B"), &[], Format::Text)).unwrap();
        assert!(
            output.document.contains("topProducts {"),
            "{}",
            output.document
        );
        assert!(!output.document.contains("me {"), "{}", output.document);
    }

    #[test]
    fn an_unknown_operation_suggests_one() {
        let query = "query TopProducts { topProducts { upc } } query Me { me { id } }";
        assert_eq!(
            error(SUPERGRAPH, query, Some("topproducts")),
            "unknown operation `topproducts` in (stdin); did you mean `TopProducts`? Its operations are `TopProducts` and `Me`"
        );
        assert_eq!(
            error(SUPERGRAPH, "{ me { id } }", Some("Me")),
            "unknown operation `Me` in (stdin); its one operation is anonymous"
        );
    }

    /// The query is validated against the API schema clients see, so a field
    /// that is `@inaccessible` cannot be selected. Every diagnostic is kept,
    /// such as apollo-compiler's about the selection set the field leaves
    /// empty.
    #[test]
    fn a_query_invalid_against_the_api_schema_is_an_error() {
        assert_eq!(
            error(SUPERGRAPH, "{ me {\n  secret } }", None),
            "query (stdin) is not valid against the supergraph's API schema: 1:3: interface, union and object types must have a subselection set; 2:3: type `User` does not have a field `secret`"
        );
    }

    #[test]
    fn a_schema_that_is_not_a_supergraph_is_an_error() {
        let error = error("type Query { a: Int }", "{ a }", None);
        assert!(
            error.starts_with("schema (stdin) is not a supergraph;"),
            "{error}"
        );
    }

    #[test]
    fn a_blank_schema_is_an_error() {
        let error = error("# nothing", "{ a }", None);
        assert!(error.starts_with("schema (stdin) is empty;"), "{error}");
    }

    /// A schema the planner rejects fails before the query is read, which
    /// may wait on stdin.
    #[test]
    fn a_bad_schema_fails_before_the_query_is_read() {
        let schema = Input::inline("type Query { a: Int }");
        let options = options(None, &[], Format::Text);
        let read = || -> Result<Input, Error> { panic!("the query is not read") };
        assert!(process(&schema, read, options, &mut Vec::new()).is_err());
    }

    #[test]
    fn a_blank_query_passes_through() {
        let output = plan_with("# nothing\n", options(None, &[], Format::Text)).unwrap();
        assert_eq!(output.document, "");
        assert!(output.notes.is_empty());
    }

    /// `Product.name` moves to the reviews subgraph under a progressive
    /// `@override`, so it is fetched from there only with its label on.
    #[test]
    fn an_override_label_moves_the_field_it_labels() {
        let query = "{ topProducts { name } }";
        let off = plan(query);
        assert!(!off.contains(r#"service: "reviews""#), "{off}");
        let labels = ["percent(50)".to_string()];
        let on = plan_with(query, options(None, &labels, Format::Text)).unwrap();
        assert!(
            on.document.contains(r#"service: "reviews""#),
            "{}",
            on.document
        );
        assert!(on.notes.is_empty(), "{:?}", on.notes);
    }

    #[test]
    fn an_unknown_override_label_gets_a_note() {
        let labels = ["percent(5)".to_string()];
        let output = plan_with("{ me { id } }", options(None, &labels, Format::Text)).unwrap();
        assert_eq!(
            output.notes,
            ["no `@override` in (stdin) has the label `percent(5)`, so it changes nothing"]
        );
    }

    /// A repeated definition is dropped with a warning, as everywhere, and
    /// the planner is given the document without it.
    #[test]
    fn a_repeated_operation_warns_and_the_first_is_planned() {
        let schema = Input::inline(SUPERGRAPH);
        let query = "query A { me { id } } query A { topProducts { upc } }";
        let mut warnings = Vec::new();
        let options = options(None, &[], Format::Text);
        let output = process(&schema, || Ok(Input::inline(query)), options, &mut warnings);
        let output = output.unwrap();
        assert!(output.document.contains("me {"), "{}", output.document);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }
}
