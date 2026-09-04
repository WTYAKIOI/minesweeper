pub mod model;
pub mod engine;
pub mod llm;
pub mod server;
pub mod cli;

pub use model::{PlayerView, Coord, RevealedCell, CellState, InferenceIR, Proof, Conclusion, CellProb, Region, RegionFeature};
pub use engine::{DeterministicEngine, MonteCarloEngine, ProbabilityEngine, RegionAnalyzer};
pub use llm::{Translator, LLMMode, LLMClient};
pub use server::create_router;
