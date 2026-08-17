use crate::util;
use graphql_parser::schema::{
    Definition as SchemaDef, Document as SchemaDoc, Field as SchemaField, TypeDefinition,
};
use std::collections::{HashMap, HashSet};

/// A target: either a bare type name (`MyType`), or a field qualified by the
/// type it is selected on (`MyType.field`).
enum Target<'t> {
    Type(&'t str),
    Field { type_name: &'t str, name: &'t str },
}

impl<'t> Target<'t> {
    fn parse(target: &'t str) -> Self {
        match target.split_once('.') {
            Some((type_name, name)) => Target::Field { type_name, name },
            None => Target::Type(target),
        }
    }
}

/// Decides whether a query selection hits one of the targets, resolving field
/// and argument types against the schema on the way.
///
/// Matching is subtype-aware in both directions: a target naming an interface
/// or union matches its implementors and members, and `Person.name` matches a
/// `name` selected on `User` as readily as `User.name` matches a `name`
/// selected on the `Person` interface. Shared by `query focus` and
/// `query strip`, which agree on what a target *is* and differ only in what
/// they do with a match.
pub struct Matcher<'s, 't> {
    type_map: HashMap<String, &'s TypeDefinition<'s, String>>,
    targets: Vec<Target<'t>>,
}

impl<'s, 't> Matcher<'s, 't> {
    pub fn new(schema: &'s SchemaDoc<'s, String>, targets: &'t [&'t str]) -> Self {
        Matcher {
            type_map: schema
                .definitions
                .iter()
                .filter_map(|def| match def {
                    SchemaDef::TypeDefinition(td) => {
                        util::schema_type_definition_name(td).map(|name| (name.clone(), td))
                    }
                    _ => None,
                })
                .collect(),
            targets: targets.iter().map(|t| Target::parse(t)).collect(),
        }
    }

    pub fn field_def(&self, parent_type: &str, name: &str) -> Option<&'s SchemaField<'s, String>> {
        self.type_map
            .get(parent_type)
            .copied()
            .and_then(util::type_fields)
            .and_then(|fields| fields.iter().find(|field| field.name == *name))
    }

    /// The name of the type a field lands on, with list and non-null wrappers
    /// stripped. `None` when the schema does not define the field, which is the
    /// case for meta fields like `__typename`.
    pub fn field_type(&self, parent_type: &str, name: &str) -> Option<&'s str> {
        self.field_def(parent_type, name)
            .and_then(|def| util::named_type(&def.field_type))
            .map(String::as_str)
    }

    /// The name of the type a field argument accepts, wrappers stripped.
    pub fn argument_type(
        &self,
        parent_type: &str,
        field_name: &str,
        argument: &str,
    ) -> Option<&'s str> {
        self.field_def(parent_type, field_name)?
            .arguments
            .iter()
            .find(|input| input.name == *argument)
            .and_then(|input| util::named_type(&input.value_type))
            .map(String::as_str)
    }

    /// True if arriving at `type_name` means arriving at one of the targeted
    /// types. Subtypes count: reaching a `User` is reaching a `Person` when
    /// `User implements Person`.
    pub fn is_type_target(&self, type_name: &str) -> bool {
        self.targets.iter().any(|target| match target {
            Target::Type(target) => self.is_subtype_of(type_name, target),
            Target::Field { .. } => false,
        })
    }

    /// True if `name` selected on `parent_type` is one of the targeted fields.
    pub fn is_field_target(&self, parent_type: &str, name: &str) -> bool {
        self.targets.iter().any(|target| match target {
            Target::Field {
                type_name: target_type,
                name: target_name,
            } => {
                *target_name == name
                    && (self.is_subtype_of(parent_type, target_type)
                        || self.is_subtype_of(target_type, parent_type))
            }
            Target::Type(_) => false,
        })
    }

    /// True if `sub` is `sup`, implements the interface `sup`, or is a member of
    /// the union `sup`.
    fn is_subtype_of(&self, sub: &str, sup: &str) -> bool {
        if sub == sup {
            return true;
        }
        match self.type_map.get(sup) {
            Some(TypeDefinition::Union(union_type)) => {
                union_type.types.iter().any(|member| member == sub)
            }
            Some(TypeDefinition::Interface(_)) => self.implements(sub, sup, &mut HashSet::new()),
            _ => false,
        }
    }

    /// Walks the interface hierarchy, since interfaces may themselves implement
    /// interfaces. `seen` breaks cycles in invalid schemas.
    fn implements(&self, type_name: &str, interface: &str, seen: &mut HashSet<String>) -> bool {
        if !seen.insert(type_name.to_string()) {
            return false;
        }
        let interfaces = match self.type_map.get(type_name) {
            Some(TypeDefinition::Object(obj)) => &obj.implements_interfaces,
            Some(TypeDefinition::Interface(iface)) => &iface.implements_interfaces,
            _ => return false,
        };
        interfaces
            .iter()
            .any(|i| i == interface || self.implements(i, interface, seen))
    }
}
