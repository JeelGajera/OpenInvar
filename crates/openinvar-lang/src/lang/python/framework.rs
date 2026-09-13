//! Framework conventions that create dependencies without a call.
//!
//! A Pydantic or Django model's fields are its public contract even though
//! nothing calls them, and a `@dataclass` generates `__init__` from its
//! annotations. Recognising the base classes and decorators involved is what
//! lets impact analysis answer "what breaks if I rename this field" for the
//! frameworks most Python services are built on.
//!
//! This module previously scanned for `__all__` with string splitting; that
//! job now belongs to the extractor, which reads it from the AST.

/// Base classes that mark a class as a data model with meaningful fields.
const MODEL_BASES: &[&str] = &[
    "BaseModel", // Pydantic
    "BaseSettings",
    "Model", // Django
    "AbstractUser",
    "AbstractBaseUser",
    "Serializer", // Django REST Framework
    "ModelSerializer",
    "Schema", // Marshmallow / Ninja
    "TypedDict",
    "NamedTuple",
];

/// Base classes that make a class an interface rather than an implementation.
const INTERFACE_BASES: &[&str] = &["Protocol", "ABC", "ABCMeta", "Generic"];

/// Decorators that generate behaviour from a class's annotations.
const MODEL_DECORATORS: &[&str] = &[
    "dataclass",
    "attrs",
    "define",
    "frozen",
    "pydantic_dataclass",
];

/// What a class's bases and decorators say about its role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassRole {
    /// Declares an interface others implement.
    Interface,
    /// Fields form a data contract (Pydantic, Django, dataclass).
    Model,
    /// An ordinary class.
    Plain,
}

/// Classify a class from its base-class names and decorator names.
pub fn classify(bases: &[String], decorators: &[String]) -> ClassRole {
    if bases
        .iter()
        .any(|b| INTERFACE_BASES.contains(&last_segment(b)))
    {
        return ClassRole::Interface;
    }
    if bases.iter().any(|b| MODEL_BASES.contains(&last_segment(b))) {
        return ClassRole::Model;
    }
    if decorators
        .iter()
        .any(|d| MODEL_DECORATORS.contains(&last_segment(d)))
    {
        return ClassRole::Model;
    }
    ClassRole::Plain
}

/// The final component of a dotted name: `models.Model` → `Model`.
fn last_segment(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pydantic_and_django_models_are_recognised() {
        assert_eq!(classify(&["BaseModel".into()], &[]), ClassRole::Model);
        assert_eq!(
            classify(&["models.Model".into()], &[]),
            ClassRole::Model,
            "dotted base classes are the common Django spelling"
        );
    }

    #[test]
    fn dataclasses_are_recognised_by_decorator() {
        assert_eq!(classify(&[], &["dataclass".into()]), ClassRole::Model);
        assert_eq!(
            classify(&[], &["dataclasses.dataclass".into()]),
            ClassRole::Model
        );
    }

    #[test]
    fn protocols_and_abcs_are_interfaces() {
        assert_eq!(classify(&["Protocol".into()], &[]), ClassRole::Interface);
        assert_eq!(classify(&["ABC".into()], &[]), ClassRole::Interface);
    }

    #[test]
    fn interface_wins_over_model_when_both_apply() {
        assert_eq!(
            classify(&["Protocol".into(), "BaseModel".into()], &[]),
            ClassRole::Interface
        );
    }

    #[test]
    fn ordinary_classes_are_plain() {
        assert_eq!(classify(&["object".into()], &[]), ClassRole::Plain);
        assert_eq!(classify(&[], &[]), ClassRole::Plain);
    }
}
