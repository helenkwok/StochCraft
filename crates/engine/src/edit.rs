//! Changes to a model, each with its exact inverse.
//!
//! Every editing command becomes an [`Op`]; applying it returns the [`Op`] that undoes it.
//! Applying that inverse returns the original again, which is how redo works. Inverses are kept
//! as typed values rather than JSON, so they restore the model exactly.

use stochcraft_model::{Connection, Element, ElementId, ElementKind, Experiment, Model, Property};

use crate::{CommandError, ErrorCode};

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Replace the whole model.
    SetModel(Box<Model>),
    RenameModel(String),
    /// Insert an element at `index`, then each connection at its index (ascending).
    AddElement {
        index: usize,
        element: Element,
        connections: Vec<(usize, Connection)>,
    },
    /// Remove an element and every connection to or from it.
    RemoveElement(ElementId),
    SetKind(ElementId, ElementKind),
    RenameElement(ElementId, String),
    /// Replace an element's properties.
    SetProperties(ElementId, Vec<Property>),
    Connect(usize, Connection),
    Disconnect(ElementId),
    /// Set a connection's probability and condition.
    SetRoute(ElementId, Option<f64>, Option<String>),
    SetExperiment(Experiment),
}

fn not_found(what: &str, id: &ElementId) -> CommandError {
    CommandError::new(ErrorCode::NotFound, format!("no {what} with id `{id}`"))
}

fn element_index(model: &Model, id: &ElementId) -> Result<usize, CommandError> {
    model
        .elements
        .iter()
        .position(|e| &e.id == id)
        .ok_or_else(|| not_found("element", id))
}

/// Applies `op` to `model` and returns its inverse. On error the model is unchanged.
pub fn apply(model: &mut Model, op: Op) -> Result<Op, CommandError> {
    Ok(match op {
        Op::SetModel(m) => Op::SetModel(Box::new(std::mem::replace(model, *m))),
        Op::RenameModel(name) => Op::RenameModel(std::mem::replace(&mut model.name, name)),
        Op::AddElement {
            index,
            element,
            connections,
        } => {
            let id = element.id.clone();
            let index = index.min(model.elements.len());
            model.elements.insert(index, element);
            for (i, c) in connections {
                let i = i.min(model.connections.len());
                model.connections.insert(i, c);
            }
            Op::RemoveElement(id)
        }
        Op::RemoveElement(id) => {
            let index = element_index(model, &id)?;
            let element = model.elements.remove(index);
            let mut removed = Vec::new();
            let mut kept = Vec::new();
            for (i, c) in std::mem::take(&mut model.connections)
                .into_iter()
                .enumerate()
            {
                if c.from == id || c.to == id {
                    removed.push((i, c));
                } else {
                    kept.push(c);
                }
            }
            model.connections = kept;
            Op::AddElement {
                index,
                element,
                connections: removed,
            }
        }
        Op::SetKind(id, kind) => {
            let index = element_index(model, &id)?;
            let e = model
                .elements
                .get_mut(index)
                .ok_or_else(|| not_found("element", &id))?;
            Op::SetKind(id, std::mem::replace(&mut e.kind, kind))
        }
        Op::RenameElement(id, name) => {
            let index = element_index(model, &id)?;
            let e = model
                .elements
                .get_mut(index)
                .ok_or_else(|| not_found("element", &id))?;
            Op::RenameElement(id, std::mem::replace(&mut e.name, name))
        }
        Op::SetProperties(id, properties) => {
            let index = element_index(model, &id)?;
            let e = model
                .elements
                .get_mut(index)
                .ok_or_else(|| not_found("element", &id))?;
            Op::SetProperties(id, std::mem::replace(&mut e.properties, properties))
        }
        Op::Connect(index, c) => {
            let id = c.id.clone();
            let index = index.min(model.connections.len());
            model.connections.insert(index, c);
            Op::Disconnect(id)
        }
        Op::Disconnect(id) => {
            let index = model
                .connections
                .iter()
                .position(|c| c.id == id)
                .ok_or_else(|| not_found("connection", &id))?;
            let c = model.connections.remove(index);
            Op::Connect(index, c)
        }
        Op::SetRoute(id, probability, condition) => {
            let c = model
                .connections
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or_else(|| not_found("connection", &id))?;
            Op::SetRoute(
                id,
                std::mem::replace(&mut c.probability, probability),
                std::mem::replace(&mut c.condition, condition),
            )
        }
        Op::SetExperiment(x) => Op::SetExperiment(std::mem::replace(&mut model.experiment, x)),
    })
}
