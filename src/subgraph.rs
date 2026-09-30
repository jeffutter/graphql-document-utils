//! `schema subgraph` and `schema split`: the subgraph schemas a supergraph was
//! composed from, read back out of its join directives.
//!
//! A supergraph records, for each type, field, interface, union member, and
//! enum value, which subgraphs ("graphs") have it, and for each graph the
//! federation directives that decide how it is resolved: its keys, and which
//! fields it declares `@external`, `@requires`, `@provides`, or `@override`s.
//! Extraction projects the supergraph onto one graph and writes those back as
//! the federation directives a subgraph applies, under a
//! `@link(url: "https://specs.apollo.dev/federation/v2.N")` importing exactly
//! the ones used. Composing the extracted subgraphs gives the supergraph back.
//!
//! A supergraph does not record everything its subgraphs said, so some of it
//! is rebuilt: `@shareable` goes on a field more than one graph resolves,
//! which is when composition requires it, and on a type rather than its fields
//! when all of them are. What it cannot rebuild is lost: how a subgraph named
//! its root types, arguments one subgraph declared differently than composition
//! merged them, and custom directives it did not compose.

use crate::{
    error::Error,
    input::Input,
    query_target,
    supergraph::{
        enum_argument, is_true, string_argument, type_directives, FieldSet, Graph, Supergraph,
    },
    util, Output,
};
use graphql_parser::{
    query::{Selection, Value},
    schema::{
        Definition, Directive, DirectiveLocation, Document, EnumType, Field, InputObjectType,
        InputValue, InterfaceType, ObjectType, ScalarType, SchemaDefinition, TypeDefinition,
        UnionType,
    },
    Pos,
};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::Path,
};

/// Prints the subgraph of the supergraph `schema` named `name`, as
/// `@join__graph(name:)` names it.
///
/// A blank schema passes through as empty, as it does through every schema
/// command, and its name goes unchecked, as there is nothing to look it up
/// in. A schema that is not a supergraph is `Error::NotASupergraph`, and a
/// Federation 1 one `Error::MalformedSupergraph`. A name no subgraph has is
/// `Error::UnknownSubgraph`, with a did-you-mean (see `query_target::suggest`)
/// and the names it has. A subgraph with no types prints nothing, with a note.
///
/// What the supergraph records that extraction does not carry into the
/// subgraph adds a warning to `warnings` (see `Source::read`).
pub fn process(schema: &Input, name: &str, warnings: &mut Vec<String>) -> Result<Output, Error> {
    let Some(source) = Source::read(schema, warnings)? else {
        return Ok(Output::default());
    };
    let Some(graph) = source.graphs.iter().find(|g| g.name == name) else {
        let names: Vec<&str> = source.graphs.iter().map(|g| g.name.as_str()).collect();
        return Err(Error::UnknownSubgraph {
            name: name.to_string(),
            origin: schema.origin().clone(),
            suggestion: query_target::suggest(name, names.iter().copied()),
            available: names.iter().map(|name| name.to_string()).collect(),
        });
    };

    let document = source.extract(graph);
    let notes = if document.is_empty() {
        vec![format!("subgraph `{name}` has no types; output is empty")]
    } else {
        Vec::new()
    };
    Ok(Output { document, notes })
}

/// Writes each subgraph of the supergraph `schema` to `<dir>/<name>.graphql`,
/// in the order the `join__Graph` enum lists them, creating `dir` if need be
/// and replacing those files if they exist, but touching no other. Nothing
/// goes to stdout; a note names each file written, and each subgraph with no
/// types, which gets no file.
///
/// Every subgraph is extracted, and every name checked, before anything is
/// written, so a failure leaves the directory as it was, unless writing
/// itself fails part way (`Error::WriteFile`). A blank schema writes nothing
/// and creates no directory. Otherwise it fails as `process` does.
pub fn split(schema: &Input, dir: &Path, warnings: &mut Vec<String>) -> Result<Output, Error> {
    let Some(source) = Source::read(schema, warnings)? else {
        return Ok(Output::default());
    };

    // Subgraph names come from the input and become file names, so each must
    // be a single plain path component: letters, digits, `_`, and `-` only.
    // That keeps a crafted name such as `../../x` or `/etc/x` from writing
    // outside `dir`. Two names differing only in case would write the same
    // file on a case-insensitive filesystem, one silently replacing the other,
    // so they are refused too.
    let mut seen: HashMap<String, &str> = HashMap::new();
    for graph in &source.graphs {
        let name = graph.name.as_str();
        let plain = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !plain {
            return Err(Error::MalformedSupergraph {
                origin: schema.origin().clone(),
                message: format!(
                    "subgraph name {name:?} cannot be a file name; `schema split` takes names of letters, digits, `_` and `-`, and `schema subgraph` prints any one"
                ),
            });
        }
        if let Some(other) = seen.insert(name.to_lowercase(), name) {
            return Err(Error::MalformedSupergraph {
                origin: schema.origin().clone(),
                message: format!(
                    "subgraphs `{other}` and `{name}` differ only in case, so their files would collide; use `schema subgraph` to print each"
                ),
            });
        }
    }

    let extracted: Vec<(&Graph, String)> = source
        .graphs
        .iter()
        .map(|graph| (graph, source.extract(graph)))
        .collect();

    let mut notes = Vec::new();
    let mut created = false;
    for (graph, document) in extracted {
        if document.is_empty() {
            notes.push(format!(
                "subgraph `{}` has no types; no file written",
                graph.name
            ));
            continue;
        }
        if !created {
            fs::create_dir_all(dir).map_err(|source| Error::WriteFile {
                path: dir.to_path_buf(),
                source,
            })?;
            created = true;
        }
        let path = dir.join(format!("{}.graphql", graph.name));
        fs::write(&path, format!("{}\n", document.trim_end())).map_err(|source| {
            Error::WriteFile {
                path: path.clone(),
                source,
            }
        })?;
        notes.push(format!("wrote {}", path.display()));
    }
    Ok(Output {
        document: String::new(),
        notes,
    })
}

/// The federation directives a subgraph can import, in the order extraction
/// lists them, with the federation version that first defines each.
const FEDERATION: [(&str, (u32, u32)); 14] = [
    ("key", (2, 0)),
    ("shareable", (2, 0)),
    ("external", (2, 0)),
    ("requires", (2, 0)),
    ("provides", (2, 0)),
    ("override", (2, 0)),
    ("interfaceObject", (2, 3)),
    ("inaccessible", (2, 0)),
    ("tag", (2, 0)),
    ("authenticated", (2, 5)),
    ("requiresScopes", (2, 5)),
    ("policy", (2, 6)),
    ("cost", (2, 9)),
    ("listSize", (2, 9)),
];

/// The join directives extraction reads, by their name in the join spec.
const READ: [&str; 7] = [
    "type",
    "field",
    "implements",
    "unionMember",
    "enumValue",
    "graph",
    "directive",
];

/// A supergraph read for extraction, with what every subgraph's extraction
/// consults.
struct Source<'a> {
    doc: Document<'a, String>,
    supergraph: Supergraph,
    /// Every type, extensions merged in (see `util::merged_type_definitions`).
    types: HashMap<String, TypeDefinition<'a, String>>,
    graphs: Vec<Graph>,
    field_sets: Vec<FieldSet>,
    /// The fields each graph resolves only because it `@provides` them, by
    /// graph and then type and field, which count toward `@shareable`.
    provided: HashMap<String, HashSet<(String, String)>>,
}

/// What an extraction uses of the federation spec, which decides the `@link`.
#[derive(Default)]
struct Uses {
    directives: HashSet<&'static str>,
    /// A key on an interface, which federation 2.3 introduced.
    interface_key: bool,
    /// A progressive `@override(label:)`, which federation 2.7 introduced.
    override_label: bool,
}

impl<'a> Source<'a> {
    /// Reads the supergraph `schema`, or `None` when it is blank.
    ///
    /// Warns once for each directive the supergraph applies that is linked
    /// machinery extraction does not carry into the subgraphs, and for what
    /// else it records that is not followed (see `Supergraph::untracked`).
    fn read(schema: &'a Input, warnings: &mut Vec<String>) -> Result<Option<Source<'a>>, Error> {
        let Some(doc) = schema.parse_schema(warnings)? else {
            return Ok(None);
        };
        let Some(supergraph) = Supergraph::detect(&doc) else {
            return Err(Error::NotASupergraph {
                origin: schema.origin().clone(),
            });
        };
        let malformed = |message: String| Error::MalformedSupergraph {
            origin: schema.origin().clone(),
            message,
        };
        if supergraph.is_federation_1() {
            return Err(malformed(format!(
                "it is a Federation 1 supergraph (`{}`), which `schema subgraph` and `schema split` do not read; recompose it with Federation 2",
                supergraph.join_url()
            )));
        }
        let types = util::merged_type_definitions(&doc);
        let field_sets = supergraph.field_sets(&types).map_err(malformed)?;
        let graphs = supergraph.graphs(&types);

        warnings.extend(supergraph.untracked(&types));
        let mut dropped: BTreeSet<String> = BTreeSet::new();
        let mut note = |directives: &[Directive<'_, String>]| {
            for directive in directives {
                let Some((_, element)) = supergraph.machinery_directive(&directive.name) else {
                    continue;
                };
                let read =
                    directive.name == supergraph.join(&element) && READ.contains(&element.as_str());
                if !read && supergraph.carried(&directive.name).is_none() {
                    dropped.insert(directive.name.clone());
                }
            }
        };
        for def in &doc.definitions {
            let td = match def {
                Definition::TypeDefinition(td) => td.clone(),
                Definition::TypeExtension(te) => util::extension_as_definition(te),
                Definition::SchemaDefinition(_) | Definition::DirectiveDefinition(_) => continue,
            };
            if supergraph.is_machinery_type(util::type_definition_name(&td)) {
                continue;
            }
            each_directives(&td, &mut note);
        }
        for name in dropped {
            warnings.push(format!(
                "the supergraph applies `@{name}`, which is not carried into its subgraphs"
            ));
        }

        let mut provided: HashMap<String, HashSet<(String, String)>> = HashMap::new();
        for fs in &field_sets {
            let Some(field) = &fs.field else { continue };
            let Some(owner) = types.get(&fs.type_name).and_then(util::type_fields) else {
                continue;
            };
            let Some(field) = owner.iter().find(|f| f.name == *field) else {
                continue;
            };
            for join in supergraph.joins(&field.directives, "field") {
                if string_argument(join, "provides").is_none() {
                    continue;
                }
                let Some(graph) = enum_argument(join, "graph") else {
                    continue;
                };
                provided
                    .entry(graph.to_string())
                    .or_default()
                    .extend(supergraph.field_set_fields(&types, fs, Some(graph)));
            }
        }

        Ok(Some(Source {
            doc,
            supergraph,
            types,
            graphs,
            field_sets,
            provided,
        }))
    }

    /// The subgraph `graph`, printed, or empty when it has no types.
    fn extract(&self, graph: &Graph) -> String {
        let g = graph.value.as_str();
        let sg = &self.supergraph;
        let mut uses = Uses::default();

        // Each type as the graph has it, with whether the graph wrote it as
        // an extension.
        let mut projected: HashMap<String, (TypeDefinition<'a, String>, bool)> = HashMap::new();
        for (name, td) in &self.types {
            if sg.is_machinery_type(name) {
                continue;
            }
            if let Some(projection) = self.project(td, g) {
                projected.insert(name.clone(), projection);
            }
        }

        // Unused externals and what is left empty go, until nothing more does.
        loop {
            let before = size(&projected);
            self.drop_unused_externals(g, &mut projected);
            cascade(&mut projected);
            if size(&projected) == before {
                break;
            }
        }
        if projected.is_empty() {
            return String::new();
        }

        // Directives go on last, once what survives is settled: `@shareable`
        // depends on which fields remain.
        let keys = self.keys(g);
        for (name, (td, _)) in projected.iter_mut() {
            self.federate(name, td, g, &keys, &mut uses);
        }

        // Directive definitions come over when a client can use them, or the
        // subgraph applies them.
        let mut applied: HashSet<String> = HashSet::new();
        for (td, _) in projected.values() {
            each_directives(td, |directives| {
                applied.extend(directives.iter().map(|d| d.name.clone()))
            });
        }
        let mut schema_directives = Vec::new();
        let mut roots = [None, None, None];
        let mut emitted: HashSet<String> = HashSet::new();
        let mut definitions = Vec::new();
        for def in &self.doc.definitions {
            match def {
                Definition::SchemaDefinition(sd) => {
                    schema_directives = self.directives(&sd.directives, g, &mut uses);
                    let has = |root: &Option<String>| {
                        root.clone().filter(|name| projected.contains_key(name))
                    };
                    roots = [has(&sd.query), has(&sd.mutation), has(&sd.subscription)];
                }
                Definition::DirectiveDefinition(dd) => {
                    if sg.is_machinery_directive(&dd.name) {
                        continue;
                    }
                    if applied.contains(&dd.name) || dd.locations.iter().any(is_executable) {
                        definitions.push(Definition::DirectiveDefinition(dd.clone()));
                    }
                }
                Definition::TypeDefinition(_) | Definition::TypeExtension(_) => {
                    let name = match def {
                        Definition::TypeDefinition(td) => util::type_definition_name(td),
                        Definition::TypeExtension(te) => util::type_extension_name(te),
                        _ => unreachable!(),
                    };
                    if !emitted.insert(name.clone()) {
                        continue;
                    }
                    let Some((td, extension)) = projected.remove(name) else {
                        continue;
                    };
                    if extension {
                        if let Some(te) = util::definition_as_extension(td.clone()) {
                            definitions.push(Definition::TypeExtension(te));
                            continue;
                        }
                    }
                    definitions.push(Definition::TypeDefinition(td));
                }
            }
        }
        // A supergraph without a schema definition names its roots by default.
        if !self
            .doc
            .definitions
            .iter()
            .any(|def| matches!(def, Definition::SchemaDefinition(_)))
        {
            let has = |name: &str| {
                definitions.iter().any(|def| matches!(def, Definition::TypeDefinition(td) if util::type_definition_name(td) == name)).then(|| name.to_string())
            };
            roots = [has("Query"), has("Mutation"), has("Subscription")];
        }

        schema_directives.insert(0, self.link(&uses));
        let [query, mutation, subscription] = roots;
        let position = Pos::default();
        if query.is_none() && mutation.is_none() && subscription.is_none() {
            // graphql-parser prints a schema definition with no roots, but
            // cannot read one back, and cannot print `extend schema`, the form
            // a subgraph without root types takes. It is written as the one
            // definition with the same directives it can print, then renamed.
            let carrier = Document::<String> {
                definitions: vec![Definition::TypeDefinition(TypeDefinition::Scalar(
                    ScalarType {
                        position,
                        description: None,
                        name: "S".to_string(),
                        directives: schema_directives,
                    },
                ))],
            };
            let header = carrier.to_string().replacen("scalar S", "extend schema", 1);
            return format!("{}\n\n{}", header.trim_end(), Document { definitions });
        }
        definitions.insert(
            0,
            Definition::SchemaDefinition(SchemaDefinition {
                position,
                directives: schema_directives,
                query,
                mutation,
                subscription,
            }),
        );
        Document { definitions }.to_string()
    }

    /// `td` as graph `g` has it, or `None` when `g` does not have it: only
    /// its members in `g`, their types as `g` gives them, and its directives
    /// but for the join ones, which `federate` turns into federation's.
    fn project(
        &self,
        td: &TypeDefinition<'a, String>,
        g: &str,
    ) -> Option<(TypeDefinition<'a, String>, bool)> {
        let sg = &self.supergraph;
        let own = sg.joins(type_directives(td), "type");
        let own: Vec<_> = own
            .into_iter()
            .filter(|d| enum_argument(d, "graph") == Some(g))
            .collect();
        if !sg
            .type_graphs(td, &self.graphs)
            .iter()
            .any(|graph| graph == g)
        {
            return None;
        }
        let extension = own.iter().any(|d| is_true(d, "extension"));
        let interface_object = own.iter().any(|d| is_true(d, "isInterfaceObject"));

        let fields = |fields: &[Field<'a, String>]| -> Vec<Field<'a, String>> {
            fields
                .iter()
                .filter(|f| sg.field_joins(&f.directives, true, g).is_some())
                .map(|f| Field {
                    field_type: sg.type_in(f, g),
                    ..f.clone()
                })
                .collect()
        };
        let implements = |interfaces: &[String], directives: &[Directive<'a, String>]| {
            let joins = sg.joins(directives, "implements");
            if joins.is_empty() {
                return interfaces.to_vec();
            }
            interfaces
                .iter()
                .filter(|i| {
                    joins.iter().any(|d| {
                        enum_argument(d, "graph") == Some(g)
                            && string_argument(d, "interface") == Some(i.as_str())
                    })
                })
                .cloned()
                .collect()
        };

        let projected = match td {
            TypeDefinition::Scalar(_) => td.clone(),
            TypeDefinition::Object(obj) => TypeDefinition::Object(ObjectType {
                implements_interfaces: implements(&obj.implements_interfaces, &obj.directives),
                fields: fields(&obj.fields),
                ..obj.clone()
            }),
            // An `@interfaceObject` is an object type in its subgraph, which
            // knows nothing of the interface's implementors.
            TypeDefinition::Interface(iface) if interface_object => {
                TypeDefinition::Object(ObjectType {
                    position: iface.position,
                    description: iface.description.clone(),
                    name: iface.name.clone(),
                    implements_interfaces: Vec::new(),
                    directives: iface.directives.clone(),
                    fields: fields(&iface.fields),
                })
            }
            TypeDefinition::Interface(iface) => TypeDefinition::Interface(InterfaceType {
                implements_interfaces: implements(&iface.implements_interfaces, &iface.directives),
                fields: fields(&iface.fields),
                ..iface.clone()
            }),
            TypeDefinition::Union(union) => {
                let joins = sg.joins(&union.directives, "unionMember");
                TypeDefinition::Union(UnionType {
                    types: union
                        .types
                        .iter()
                        .filter(|member| {
                            joins.is_empty()
                                || joins.iter().any(|d| {
                                    enum_argument(d, "graph") == Some(g)
                                        && string_argument(d, "member") == Some(member.as_str())
                                })
                        })
                        .cloned()
                        .collect(),
                    ..union.clone()
                })
            }
            TypeDefinition::Enum(enum_type) => TypeDefinition::Enum(EnumType {
                values: enum_type
                    .values
                    .iter()
                    .filter(|value| {
                        let joins = sg.joins(&value.directives, "enumValue");
                        joins.is_empty()
                            || joins.iter().any(|d| enum_argument(d, "graph") == Some(g))
                    })
                    .cloned()
                    .collect(),
                ..enum_type.clone()
            }),
            TypeDefinition::InputObject(input) => TypeDefinition::InputObject(InputObjectType {
                fields: input
                    .fields
                    .iter()
                    .filter(|f| sg.field_joins(&f.directives, true, g).is_some())
                    .cloned()
                    .collect(),
                ..input.clone()
            }),
        };
        Some((projected, extension))
    }

    /// The fields a field set of `g` names, by type and field: each of its
    /// keys, and the `requires` and `provides` of its fields.
    fn named_in(&self, g: &str) -> HashSet<(String, String)> {
        let sg = &self.supergraph;
        let mut named = HashSet::new();
        for fs in &self.field_sets {
            let Some(td) = self.types.get(&fs.type_name) else {
                continue;
            };
            let applies = match &fs.field {
                // A key is `g`'s when it is on one of `g`'s `@join__type`s.
                None => sg.joins(type_directives(td), "type").iter().any(|d| {
                    enum_argument(d, "graph") == Some(g)
                        && string_argument(d, "key").is_some_and(|key| {
                            crate::supergraph::parse_field_set(key).as_ref() == Some(&fs.selection)
                        })
                }),
                Some(field) => util::type_fields(td)
                    .and_then(|fields| fields.iter().find(|f| f.name == *field))
                    .and_then(|f| sg.field_joins(&f.directives, true, g))
                    .is_some_and(|joins| {
                        joins.iter().any(|d| {
                            ["requires", "provides"].iter().any(|what| {
                                string_argument(d, what).is_some_and(|text| {
                                    crate::supergraph::parse_field_set(text).as_ref()
                                        == Some(&fs.selection)
                                })
                            })
                        })
                    }),
            };
            if applies {
                named.extend(sg.field_set_fields(&self.types, fs, Some(g)));
            }
        }
        named
    }

    /// Removes the `@external` fields no field set of `g` names, since an
    /// external field is only there for one to use, and composition rejects
    /// one that is not.
    fn drop_unused_externals(
        &self,
        g: &str,
        projected: &mut HashMap<String, (TypeDefinition<'a, String>, bool)>,
    ) {
        let sg = &self.supergraph;
        let named = self.named_in(g);
        for (name, (td, _)) in projected.iter_mut() {
            let fields = match td {
                TypeDefinition::Object(obj) => &mut obj.fields,
                TypeDefinition::Interface(iface) => &mut iface.fields,
                _ => continue,
            };
            fields.retain(|f| {
                sg.resolves_in(&f.directives, true, g)
                    || named.contains(&(name.clone(), f.name.clone()))
            });
        }
    }

    /// The keys of each type in `g`, by type.
    fn keys(&self, g: &str) -> HashMap<String, Vec<(String, bool)>> {
        let sg = &self.supergraph;
        let mut keys: HashMap<String, Vec<(String, bool)>> = HashMap::new();
        for (name, td) in &self.types {
            for d in sg.joins(type_directives(td), "type") {
                if enum_argument(d, "graph") != Some(g) {
                    continue;
                }
                if let Some(key) = string_argument(d, "key") {
                    let resolvable = !matches!(
                        crate::supergraph::argument(d, "resolvable"),
                        Some(Value::Boolean(false))
                    );
                    keys.entry(name.clone())
                        .or_default()
                        .push((key.to_string(), resolvable));
                }
            }
        }
        keys
    }

    /// Replaces the supergraph's directives on `td`, and on everything in it,
    /// with those `g`'s subgraph applies: its keys, what its join directives
    /// record of each field, `@shareable` where composition needed it, and
    /// the rest as `directives` converts them.
    fn federate(
        &self,
        name: &str,
        td: &mut TypeDefinition<'a, String>,
        g: &str,
        keys: &HashMap<String, Vec<(String, bool)>>,
        uses: &mut Uses,
    ) {
        let sg = &self.supergraph;
        let source = &self.types[name];
        let position = Pos::default();
        let directive = |name: &str, arguments: Vec<(&str, Value<'a, String>)>| Directive {
            position,
            name: name.to_string(),
            arguments: arguments
                .into_iter()
                .map(|(name, value)| (name.to_string(), value))
                .collect(),
        };

        let interface_object = sg
            .joins(type_directives(source), "type")
            .iter()
            .any(|d| enum_argument(d, "graph") == Some(g) && is_true(d, "isInterfaceObject"));
        let is_interface = matches!(td, TypeDefinition::Interface(_));
        let type_keys = keys.get(name).cloned().unwrap_or_default();
        let key_fields: HashSet<String> = type_keys
            .iter()
            .filter_map(|(key, _)| crate::supergraph::parse_field_set(key))
            .flat_map(|selection| {
                selection
                    .items
                    .into_iter()
                    .filter_map(|item| match item {
                        Selection::Field(f) => Some(f.name),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        let mut type_level: Vec<Directive<'a, String>> = Vec::new();
        for (key, resolvable) in &type_keys {
            let mut arguments = vec![("fields", Value::String(key.clone()))];
            if !resolvable {
                arguments.push(("resolvable", Value::Boolean(false)));
            }
            type_level.push(directive("key", arguments));
            uses.directives.insert("key");
            uses.interface_key |= is_interface;
        }
        if interface_object {
            type_level.push(directive("interfaceObject", Vec::new()));
            uses.directives.insert("interfaceObject");
        }
        type_level.extend(self.directives(type_directives(td), g, uses));

        let mut shared = 0;
        let mut field_count = 0;
        match td {
            TypeDefinition::Object(ObjectType { fields, .. })
            | TypeDefinition::Interface(InterfaceType { fields, .. }) => {
                for field in fields.iter_mut() {
                    let joins = sg
                        .field_joins(&field.directives, true, g)
                        .unwrap_or_default();
                    let mut own: Vec<Directive<'a, String>> = Vec::new();
                    let external = joins.iter().any(|d| is_true(d, "external"));
                    if external {
                        own.push(directive("external", Vec::new()));
                        uses.directives.insert("external");
                    }
                    for (what, argument) in [("requires", "requires"), ("provides", "provides")] {
                        if let Some(fields) =
                            joins.iter().find_map(|d| string_argument(d, argument))
                        {
                            own.push(directive(
                                what,
                                vec![("fields", Value::String(fields.to_string()))],
                            ));
                            uses.directives.insert(what);
                        }
                    }
                    if let Some(from) = joins.iter().find_map(|d| string_argument(d, "override")) {
                        let mut arguments = vec![("from", Value::String(from.to_string()))];
                        if let Some(label) = joins
                            .iter()
                            .find_map(|d| string_argument(d, "overrideLabel"))
                        {
                            arguments.push(("label", Value::String(label.to_string())));
                            uses.override_label = true;
                        }
                        own.push(directive("override", arguments));
                        uses.directives.insert("override");
                    }

                    // An object field more than one subgraph resolves had to
                    // be `@shareable` in each that does. Key fields are
                    // shareable without it.
                    if !is_interface && !external {
                        field_count += 1;
                        let shareable = !key_fields.contains(&field.name)
                            && self.resolving(name, &field.name) > 1;
                        if shareable {
                            shared += 1;
                            own.push(directive("shareable", Vec::new()));
                        }
                    }

                    own.extend(self.directives(&field.directives, g, uses));
                    field.directives = own;
                    for argument in field.arguments.iter_mut() {
                        argument.directives = self.directives(&argument.directives, g, uses);
                    }
                }
                // When every field is, the type is marked instead.
                if shared > 0 && shared == field_count && field_count == fields.len() {
                    for field in fields.iter_mut() {
                        field.directives.retain(|d| d.name != "shareable");
                    }
                    type_level.push(directive("shareable", Vec::new()));
                }
                if shared > 0 {
                    uses.directives.insert("shareable");
                }
            }
            TypeDefinition::Enum(enum_type) => {
                for value in enum_type.values.iter_mut() {
                    value.directives = self.directives(&value.directives, g, uses);
                }
            }
            TypeDefinition::InputObject(input) => {
                for field in input.fields.iter_mut() {
                    field.directives = self.directives(&field.directives, g, uses);
                }
            }
            TypeDefinition::Scalar(_) | TypeDefinition::Union(_) => (),
        }

        match td {
            TypeDefinition::Scalar(t) => t.directives = type_level,
            TypeDefinition::Object(t) => t.directives = type_level,
            TypeDefinition::Interface(t) => t.directives = type_level,
            TypeDefinition::Union(t) => t.directives = type_level,
            TypeDefinition::Enum(t) => t.directives = type_level,
            TypeDefinition::InputObject(t) => t.directives = type_level,
        }
    }

    /// How many graphs resolve the field `type_name.field`: have it, not as
    /// `@external` and not while it is being overridden, or `@provides` it.
    fn resolving(&self, type_name: &str, field: &str) -> usize {
        let sg = &self.supergraph;
        let Some(td) = self.types.get(type_name) else {
            return 0;
        };
        let type_graphs = sg.type_graphs(td, &self.graphs);
        let definition =
            util::type_fields(td).and_then(|fields| fields.iter().find(|f| f.name == field));
        self.graphs
            .iter()
            .filter(|graph| {
                let g = graph.value.as_str();
                let resolves = definition
                    .and_then(|f| {
                        sg.field_joins(&f.directives, type_graphs.iter().any(|t| t == g), g)
                    })
                    .is_some_and(|joins| {
                        !joins.iter().any(|d| {
                            is_true(d, "external")
                                || is_true(d, "usedOverridden")
                                || string_argument(d, "override").is_some()
                                || string_argument(d, "overrideLabel").is_some()
                        })
                    });
                resolves
                    || self.provided.get(g).is_some_and(|set| {
                        set.contains(&(type_name.to_string(), field.to_string()))
                    })
            })
            .count()
    }

    /// `directives` as `g`'s subgraph applies them: the directives federation
    /// carries through composition (`@inaccessible`, `@tag`, and the others
    /// `Supergraph::carried` lists) under their federation names, each
    /// `@join__directive` naming `g` as the directive it records, the join
    /// spec's own dropped, and any other directive as written.
    fn directives(
        &self,
        directives: &[Directive<'a, String>],
        g: &str,
        uses: &mut Uses,
    ) -> Vec<Directive<'a, String>> {
        let sg = &self.supergraph;
        let join_directive = sg.join("directive");
        let mut converted = Vec::new();
        for d in directives {
            if d.name == join_directive {
                let names_g = match crate::supergraph::argument(d, "graphs") {
                    Some(Value::List(graphs)) => graphs
                        .iter()
                        .any(|graph| matches!(graph, Value::Enum(value) if value == g)),
                    _ => false,
                };
                let Some(name) = string_argument(d, "name").filter(|_| names_g) else {
                    continue;
                };
                let arguments = match crate::supergraph::argument(d, "args") {
                    Some(Value::Object(args)) => args
                        .iter()
                        .map(|(name, value)| (name.clone(), value.clone()))
                        .collect(),
                    _ => Vec::new(),
                };
                converted.push(Directive {
                    position: d.position,
                    name: name.to_string(),
                    arguments,
                });
            } else if let Some(name) = sg.carried(&d.name) {
                uses.directives.insert(name);
                converted.push(Directive {
                    name: name.to_string(),
                    ..d.clone()
                });
            } else if !sg.is_machinery_directive(&d.name) {
                converted.push(d.clone());
            }
        }
        converted
    }

    /// The `@link` importing the federation directives `uses` names, at the
    /// lowest federation version that defines all of them.
    fn link(&self, uses: &Uses) -> Directive<'a, String> {
        let mut version = (2, 0);
        let mut imports = Vec::new();
        for (name, since) in FEDERATION {
            if uses.directives.contains(name) {
                version = version.max(since);
                imports.push(Value::String(format!("@{name}")));
            }
        }
        if uses.interface_key {
            version = version.max((2, 3));
        }
        if uses.override_label {
            version = version.max((2, 7));
        }
        let mut arguments = vec![(
            "url".to_string(),
            Value::String(format!(
                "https://specs.apollo.dev/federation/v{}.{}",
                version.0, version.1
            )),
        )];
        if !imports.is_empty() {
            arguments.push(("import".to_string(), Value::List(imports)));
        }
        Directive {
            position: Pos::default(),
            name: "link".to_string(),
            arguments,
        }
    }
}

/// Calls `f` with every list of directives in `td`: its own, and those of its
/// fields, their arguments, its enum values, and its input fields.
fn each_directives<'a>(
    td: &TypeDefinition<'a, String>,
    mut f: impl FnMut(&[Directive<'a, String>]),
) {
    f(type_directives(td));
    let input_values = |values: &[InputValue<'a, String>],
                        f: &mut dyn FnMut(&[Directive<'a, String>])| {
        for value in values {
            f(&value.directives);
        }
    };
    match td {
        TypeDefinition::Object(ObjectType { fields, .. })
        | TypeDefinition::Interface(InterfaceType { fields, .. }) => {
            for field in fields {
                f(&field.directives);
                input_values(&field.arguments, &mut f);
            }
        }
        TypeDefinition::Enum(enum_type) => {
            for value in &enum_type.values {
                f(&value.directives);
            }
        }
        TypeDefinition::InputObject(input) => input_values(&input.fields, &mut f),
        TypeDefinition::Scalar(_) | TypeDefinition::Union(_) => (),
    }
}

/// How much `projected` holds, which only shrinks, so an unchanged size means
/// nothing was removed.
fn size(projected: &HashMap<String, (TypeDefinition<'_, String>, bool)>) -> usize {
    projected
        .values()
        .map(|(td, _)| {
            1 + match td {
                TypeDefinition::Object(obj) => obj.fields.len() + obj.implements_interfaces.len(),
                TypeDefinition::Interface(iface) => {
                    iface.fields.len() + iface.implements_interfaces.len()
                }
                TypeDefinition::Union(union) => union.types.len(),
                TypeDefinition::Enum(enum_type) => enum_type.values.len(),
                TypeDefinition::InputObject(input) => input.fields.len(),
                TypeDefinition::Scalar(_) => 0,
            } + util::type_fields(td)
                .map(|fields| fields.iter().map(|f| f.arguments.len()).sum::<usize>())
                .unwrap_or(0)
        })
        .sum()
}

/// Removes every type left with nothing (an object, interface, or input
/// object with no fields, a union with no members, an enum with no values),
/// and everything naming a type the graph does not have: fields, arguments,
/// and input fields of that type, `implements` clauses, and union members.
/// Repeats until nothing more goes, since each removal can empty another type.
fn cascade(projected: &mut HashMap<String, (TypeDefinition<'_, String>, bool)>) {
    loop {
        let empty: Vec<String> = projected
            .iter()
            .filter(|(_, (td, _))| match td {
                TypeDefinition::Object(obj) => obj.fields.is_empty(),
                TypeDefinition::Interface(iface) => iface.fields.is_empty(),
                TypeDefinition::Union(union) => union.types.is_empty(),
                TypeDefinition::Enum(enum_type) => enum_type.values.is_empty(),
                TypeDefinition::InputObject(input) => input.fields.is_empty(),
                TypeDefinition::Scalar(_) => false,
            })
            .map(|(name, _)| name.clone())
            .collect();
        for name in &empty {
            projected.remove(name);
        }

        let names: HashSet<String> = projected.keys().cloned().collect();
        let known =
            |name: &String| names.contains(name) || util::BUILT_IN_SCALARS.contains(&name.as_str());
        let typed =
            |ty: &graphql_parser::schema::Type<'_, String>| util::named_type(ty).is_none_or(known);
        let mut changed = false;
        for (td, _) in projected.values_mut() {
            match td {
                TypeDefinition::Object(ObjectType {
                    implements_interfaces,
                    fields,
                    ..
                })
                | TypeDefinition::Interface(InterfaceType {
                    implements_interfaces,
                    fields,
                    ..
                }) => {
                    let counts = (
                        implements_interfaces.len(),
                        fields.len(),
                        fields.iter().map(|f| f.arguments.len()).sum::<usize>(),
                    );
                    implements_interfaces.retain(|i| known(i));
                    fields.retain(|f| typed(&f.field_type));
                    for field in fields.iter_mut() {
                        field.arguments.retain(|a| typed(&a.value_type));
                    }
                    changed |= counts
                        != (
                            implements_interfaces.len(),
                            fields.len(),
                            fields.iter().map(|f| f.arguments.len()).sum::<usize>(),
                        );
                }
                TypeDefinition::Union(union) => {
                    let count = union.types.len();
                    union.types.retain(|m| known(m));
                    changed |= count != union.types.len();
                }
                TypeDefinition::InputObject(input) => {
                    let count = input.fields.len();
                    input.fields.retain(|f| typed(&f.value_type));
                    changed |= count != input.fields.len();
                }
                TypeDefinition::Scalar(_) | TypeDefinition::Enum(_) => (),
            }
        }
        if empty.is_empty() && !changed {
            break;
        }
    }
}

/// Whether a directive can be applied in an operation, which makes its
/// definition part of what every subgraph's clients can use.
fn is_executable(location: &DirectiveLocation) -> bool {
    matches!(
        location,
        DirectiveLocation::Query
            | DirectiveLocation::Mutation
            | DirectiveLocation::Subscription
            | DirectiveLocation::Field
            | DirectiveLocation::FragmentDefinition
            | DirectiveLocation::FragmentSpread
            | DirectiveLocation::InlineFragment
            | DirectiveLocation::VariableDefinition
    )
}

#[cfg(test)]
mod tests {
    use super::{process, split};
    use crate::{error::Error, input::Input, sort};
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use std::fs;

    const SUPERGRAPH: &str = include_str!("../tests/fixtures/supergraph.graphql");

    /// A Federation 2 supergraph of graphs `a` and `b`, linking `links` too,
    /// around `body`.
    fn supergraph(links: &str, body: &str) -> String {
        format!(
            "schema @link(url: \"https://specs.apollo.dev/link/v1.0\") \
             @link(url: \"https://specs.apollo.dev/join/v0.5\", for: EXECUTION) {links} \
             {{ query: Query }}\n\
             enum join__Graph {{\n  A @join__graph(name: \"a\", url: \"http://a\")\n  \
             B @join__graph(name: \"b\", url: \"http://b\")\n}}\n{body}"
        )
    }

    /// The subgraph `name` of `schema`, and the warnings extraction gave.
    fn extract(schema: &str, name: &str) -> (String, Vec<String>) {
        let mut warnings = Vec::new();
        let output = process(&Input::inline(schema), name, &mut warnings).unwrap();
        assert_eq!(output.notes, Vec::<String>::new());
        (output.document, warnings)
    }

    /// `schema` with its definitions sorted and each one's lines too, so two
    /// schemas compare equal when they define the same members, whatever
    /// order they list them in.
    fn normalized(schema: &str) -> Vec<Vec<String>> {
        sort::process(&Input::inline(schema))
            .unwrap()
            .split("\n\n")
            .map(|definition| {
                let mut lines: Vec<String> = definition.lines().map(str::to_string).collect();
                lines.sort();
                lines
            })
            .collect()
    }

    /// `document` with its first definition, the schema definition or
    /// `extend schema`, split off.
    fn header(document: &str) -> (&str, &str) {
        document.split_once("\n\n").unwrap()
    }

    #[test]
    fn extracts_the_subgraphs_the_supergraph_was_composed_from() {
        // The fixture's subgraphs link the federation version they were
        // written against; extraction links the lowest one they need.
        let links = [
            (
                "accounts",
                r#"@link(url: "https://specs.apollo.dev/federation/v2.0", import: ["@key", "@shareable", "@inaccessible"])"#,
            ),
            (
                "products",
                r#"@link(url: "https://specs.apollo.dev/federation/v2.3", import: ["@key", "@shareable"])"#,
            ),
            (
                "reviews",
                r#"@link(url: "https://specs.apollo.dev/federation/v2.7", import: ["@key", "@external", "@requires", "@provides", "@override", "@interfaceObject"])"#,
            ),
        ];
        for (name, link) in links {
            let (extracted, warnings) = extract(SUPERGRAPH, name);
            assert_eq!(warnings, Vec::<String>::new());
            let (schema, body) = header(&extracted);
            assert!(schema.starts_with(&format!("schema {link} {{")), "{schema}");

            let source =
                fs::read_to_string(format!("tests/fixtures/subgraphs/{name}.graphql")).unwrap();
            let (_, source_body) = source.split_once(")\n\n").unwrap();
            assert_eq!(normalized(body), normalized(source_body), "{name}");
        }
    }

    #[test]
    fn a_type_a_graph_extends_is_an_extension_in_it() {
        let schema = supergraph(
            "",
            indoc! {r#"
                type Query @join__type(graph: A) @join__type(graph: B) {
                  user: User @join__field(graph: A)
                  me: User @join__field(graph: B)
                }
                type User @join__type(graph: A, key: "id") @join__type(graph: B, key: "id", extension: true) {
                  id: ID!
                  name: String @join__field(graph: A)
                }
            "#},
        );
        let (b, _) = extract(&schema, "b");
        assert_eq!(
            header(&b).1,
            indoc! {r#"
                type Query {
                  me: User
                }

                extend type User @key(fields: "id") {
                  id: ID!
                }
            "#}
        );
    }

    #[test]
    fn a_field_takes_the_type_its_graph_gave_it() {
        let schema = supergraph(
            "",
            indoc! {r#"
                type Query @join__type(graph: A) @join__type(graph: B) {
                  count: Int! @join__field(graph: A, type: "Int!") @join__field(graph: B, type: "Int")
                }
            "#},
        );
        let (a, _) = extract(&schema, "a");
        let (b, _) = extract(&schema, "b");
        // Both resolve it, so each marks it shareable, on the type as it is
        // the only field.
        assert_eq!(header(&a).1, "type Query @shareable {\n  count: Int!\n}\n");
        assert_eq!(header(&b).1, "type Query @shareable {\n  count: Int\n}\n");
    }

    #[test]
    fn a_join_directive_becomes_the_directive_it_records_in_the_graphs_it_names() {
        let schema = supergraph(
            "",
            indoc! {r#"
                type Query @join__type(graph: A) @join__type(graph: B) @join__directive(graphs: [A], name: "custom", args: {level: 2}) {
                  a: Int @join__field(graph: A)
                  b: Int @join__field(graph: B)
                }
            "#},
        );
        let (a, _) = extract(&schema, "a");
        let (b, _) = extract(&schema, "b");
        assert_eq!(
            header(&a).1,
            "type Query @custom(level: 2) {\n  a: Int\n}\n"
        );
        assert_eq!(header(&b).1, "type Query {\n  b: Int\n}\n");
    }

    #[test]
    fn a_carried_directive_is_applied_and_imported_under_its_federation_name() {
        let schema = supergraph(
            r#"@link(url: "https://specs.apollo.dev/tag/v0.3", as: "label")"#,
            indoc! {r#"
                directive @label(name: String!) repeatable on FIELD_DEFINITION | OBJECT
                type Query @join__type(graph: A) @join__type(graph: B) {
                  a: Int @label(name: "public")
                  b: Int @join__field(graph: B)
                }
            "#},
        );
        // The supergraph does not say which subgraph applied it, so every
        // one that has the field does.
        let (a, _) = extract(&schema, "a");
        assert_eq!(
            a,
            indoc! {r#"
                schema @link(url: "https://specs.apollo.dev/federation/v2.0", import: ["@shareable", "@tag"]) {
                  query: Query
                }

                type Query @shareable {
                  a: Int @tag(name: "public")
                }
            "#}
        );
        let (b, _) = extract(&schema, "b");
        assert_eq!(
            header(&b).1,
            "type Query {\n  a: Int @shareable @tag(name: \"public\")\n  b: Int\n}\n"
        );
    }

    #[test]
    fn an_unused_external_field_goes_and_what_it_empties_with_it() {
        let schema = supergraph(
            "",
            indoc! {r#"
                type Query @join__type(graph: A) @join__type(graph: B) {
                  a: Thing @join__field(graph: A)
                  b: Thing @join__field(graph: B)
                  c: Int @join__field(graph: B)
                }
                type Thing @join__type(graph: A) @join__type(graph: B) {
                  x: Int @join__field(graph: A) @join__field(graph: B, external: true)
                }
            "#},
        );
        // In `b`, `Thing.x` is external and no field set names it, so it
        // goes; that empties `Thing`, which takes `Query.b` with it.
        let (b, _) = extract(&schema, "b");
        assert_eq!(header(&b).1, "type Query {\n  c: Int\n}\n");
        let (a, _) = extract(&schema, "a");
        assert_eq!(
            header(&a).1,
            "type Query {\n  a: Thing\n}\n\ntype Thing {\n  x: Int\n}\n"
        );
    }

    #[test]
    fn a_subgraph_without_root_types_extends_the_schema() {
        let schema = supergraph(
            "",
            indoc! {r#"
                type Query @join__type(graph: A) {
                  a: Int
                }
                type Thing @join__type(graph: B, key: "id") {
                  id: ID!
                }
            "#},
        );
        let (b, _) = extract(&schema, "b");
        assert_eq!(
            b,
            indoc! {r#"
                extend schema @link(url: "https://specs.apollo.dev/federation/v2.0", import: ["@key"])

                type Thing @key(fields: "id") {
                  id: ID!
                }
            "#}
        );
    }

    #[test]
    fn a_subgraph_using_no_federation_directive_imports_none() {
        let schema = supergraph("", "type Query @join__type(graph: A) {\n  a: Int\n}\n");
        let (a, _) = extract(&schema, "a");
        assert_eq!(
            header(&a).0,
            "schema @link(url: \"https://specs.apollo.dev/federation/v2.0\") {\n  query: Query\n}"
        );
    }

    #[test]
    fn a_linked_directive_extraction_does_not_carry_is_a_warning() {
        let schema = supergraph(
            r#"@link(url: "https://example.com/audit/v1.0")"#,
            indoc! {r#"
                directive @audit on FIELD_DEFINITION
                type Query @join__type(graph: A) {
                  a: Int @audit
                }
            "#},
        );
        let (a, warnings) = extract(&schema, "a");
        assert_eq!(header(&a).1, "type Query {\n  a: Int\n}\n");
        assert_eq!(
            warnings,
            ["the supergraph applies `@audit`, which is not carried into its subgraphs"]
        );
    }

    #[test]
    fn a_schema_that_is_not_a_supergraph_is_an_error() {
        let error = process(
            &Input::inline("type Query { a: Int }"),
            "a",
            &mut Vec::new(),
        );
        assert!(
            matches!(error, Err(Error::NotASupergraph { .. })),
            "{error:?}"
        );
    }

    #[test]
    fn a_federation_1_supergraph_is_an_error() {
        let schema = indoc! {r#"
            schema @core(feature: "https://specs.apollo.dev/core/v0.2") @core(feature: "https://specs.apollo.dev/join/v0.1", for: EXECUTION) { query: Query }
            enum join__Graph { A @join__graph(name: "a", url: "http://a") }
            type Query { a: Int }
        "#};
        let error = process(&Input::inline(schema), "a", &mut Vec::new()).unwrap_err();
        assert!(
            matches!(&error, Error::MalformedSupergraph { message, .. } if message.contains("Federation 1")),
            "{error:?}"
        );
    }

    #[test]
    fn an_unknown_subgraph_suggests_one_and_lists_them_all() {
        let error = process(&Input::inline(SUPERGRAPH), "ACCOUNTS", &mut Vec::new()).unwrap_err();
        let Error::UnknownSubgraph {
            name,
            suggestion,
            available,
            ..
        } = error
        else {
            panic!("{error:?}");
        };
        assert_eq!(name, "ACCOUNTS");
        assert_eq!(suggestion.as_deref(), Some("accounts"));
        assert_eq!(available, ["accounts", "products", "reviews"]);
    }

    #[test]
    fn a_subgraph_with_no_types_prints_nothing_with_a_note() {
        let schema = supergraph("", "type Query @join__type(graph: A) {\n  a: Int\n}\n");
        let output = process(&Input::inline(&schema), "b", &mut Vec::new()).unwrap();
        assert_eq!(output.document, "");
        assert_eq!(output.notes, ["subgraph `b` has no types; output is empty"]);
    }

    #[test]
    fn a_blank_schema_passes_through() {
        let output = process(&Input::inline("\n# nothing\n"), "any", &mut Vec::new()).unwrap();
        assert_eq!(output.document, "");
        assert_eq!(output.notes, Vec::<String>::new());
    }

    #[test]
    fn split_writes_each_subgraph_to_a_file_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("subgraphs");
        let output = split(&Input::inline(SUPERGRAPH), &out, &mut Vec::new()).unwrap();

        assert_eq!(output.document, "");
        let names = ["accounts", "products", "reviews"];
        let paths: Vec<_> = names
            .iter()
            .map(|name| out.join(format!("{name}.graphql")))
            .collect();
        assert_eq!(
            output.notes,
            paths
                .iter()
                .map(|path| format!("wrote {}", path.display()))
                .collect::<Vec<_>>()
        );
        let mut written: Vec<_> = fs::read_dir(&out)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        written.sort();
        assert_eq!(written, paths);
        for (name, path) in names.iter().zip(&paths) {
            let (extracted, _) = extract(SUPERGRAPH, name);
            assert_eq!(fs::read_to_string(path).unwrap(), extracted);
        }
    }

    /// Splitting again replaces the files of its subgraphs, and leaves every
    /// other file as it was.
    #[test]
    fn split_replaces_its_own_files_and_no_other() {
        let dir = tempfile::tempdir().unwrap();
        let stale = dir.path().join("accounts.graphql");
        let other = dir.path().join("notes.graphql");
        fs::write(&stale, "type Stale { a: Int }\n").unwrap();
        fs::write(&other, "type Other { a: Int }\n").unwrap();

        split(&Input::inline(SUPERGRAPH), dir.path(), &mut Vec::new()).unwrap();

        let (accounts, _) = extract(SUPERGRAPH, "accounts");
        assert_eq!(fs::read_to_string(&stale).unwrap(), accounts);
        assert_eq!(
            fs::read_to_string(&other).unwrap(),
            "type Other { a: Int }\n"
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 4);
    }

    #[test]
    fn split_skips_a_subgraph_with_no_types_with_a_note() {
        let dir = tempfile::tempdir().unwrap();
        let schema = supergraph("", "type Query @join__type(graph: A) {\n  a: Int\n}\n");
        let output = split(&Input::inline(&schema), dir.path(), &mut Vec::new()).unwrap();
        assert_eq!(
            output.notes,
            [
                format!("wrote {}", dir.path().join("a.graphql").display()),
                "subgraph `b` has no types; no file written".to_string(),
            ]
        );
    }

    /// A supergraph of the two subgraphs `first` and `second`, each with a
    /// type.
    fn named(first: &str, second: &str) -> String {
        format!(
            "schema @link(url: \"https://specs.apollo.dev/link/v1.0\") \
             @link(url: \"https://specs.apollo.dev/join/v0.5\", for: EXECUTION) {{ query: Query }}\n\
             enum join__Graph {{\n  A @join__graph(name: {first:?}, url: \"http://a\")\n  \
             B @join__graph(name: {second:?}, url: \"http://b\")\n}}\n\
             type Query @join__type(graph: A) @join__type(graph: B) {{ a: Int }}\n"
        )
    }

    #[test]
    fn split_refuses_a_name_that_is_not_a_plain_file_name_and_writes_nothing() {
        for bad in ["../escape", "/etc/escape", "a.b", "a b", ""] {
            let dir = tempfile::tempdir().unwrap();
            let out = dir.path().join("out");
            let error =
                split(&Input::inline(&named("fine", bad)), &out, &mut Vec::new()).unwrap_err();
            assert!(
                matches!(&error, Error::MalformedSupergraph { message, .. } if message.contains("cannot be a file name")),
                "{bad:?}: {error:?}"
            );
            assert!(!out.exists(), "{bad:?}");
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0, "{bad:?}");
        }
        // `schema subgraph` prints any of them, as nothing is written.
        let output = process(
            &Input::inline(&named("fine", "../escape")),
            "../escape",
            &mut Vec::new(),
        );
        assert!(output.unwrap().document.contains("type Query"));
    }

    #[test]
    fn split_refuses_names_that_differ_only_in_case() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let error = split(
            &Input::inline(&named("users", "Users")),
            &out,
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(
            matches!(&error, Error::MalformedSupergraph { message, .. } if message.contains("differ only in case")),
            "{error:?}"
        );
        assert!(!out.exists());
    }

    #[test]
    fn split_of_a_blank_schema_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let output = split(&Input::inline(""), &out, &mut Vec::new()).unwrap();
        assert_eq!(output.document, "");
        assert_eq!(output.notes, Vec::<String>::new());
        assert!(!out.exists());
    }

    #[test]
    fn split_into_a_directory_it_cannot_make_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        fs::write(&file, "").unwrap();
        let out = file.join("out");
        let error = split(&Input::inline(SUPERGRAPH), &out, &mut Vec::new()).unwrap_err();
        assert!(
            matches!(&error, Error::WriteFile { path, .. } if *path == out),
            "{error:?}"
        );
    }
}
