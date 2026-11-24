//! Parametric low-poly 3D mesh generation for StochCraft's layout view.
//!
//! This crate produces plain vertex and index buffers independent of any GPU or windowing
//! backend, so the same geometry drives the 3D viewport and headless tests.

pub mod mesh;

pub use mesh::kit;
pub use mesh::{
    Agv, ConveyorKind, CurvedConveyor, Forklift, Machine, Mesh, Operator, Rack, SpiralConveyor,
    StraightConveyor, Tote, agv, box_mesh, curved_conveyor, forklift, machine, operator, rack,
    spiral_conveyor, straight_conveyor, tote,
};
