pub mod coord;
pub mod player_view;
pub mod inference_ir;

pub use coord::Coord;
pub use player_view::{PlayerView, RevealedCell, CellState};
pub use inference_ir::{InferenceIR, Proof, Conclusion, CellProb, Region, RegionFeature,
    FlagStatus, FlagVerificationResult, FlagVerifyStatus};
