use rayon::prelude::*;
use crate::kit::common::JobOutcome;
use crate::kit::contracts::{validate_cardinality, validate_path, ToolError};
use std::path::PathBuf;

pub struct BatchRunner;

impl BatchRunner {
    pub fn run<F>(
        command: &'static str,
        inputs: Vec<PathBuf>,
        job: F,
    ) -> Vec<JobOutcome>
    where
        F: Fn(PathBuf) -> JobOutcome + Sync + Send,
    {
        if let Err(error) = validate_cardinality(command, &inputs) {
            return failure_outcomes(inputs, error);
        }

        inputs.into_par_iter().map(|input| match validate_path(command, &input) {
            Ok(()) => job(input),
            Err(error) => JobOutcome::failure(input, error),
        }).collect()
    }
}

fn failure_outcomes(inputs: Vec<PathBuf>, error: ToolError) -> Vec<JobOutcome> {
    if inputs.is_empty() {
        return vec![JobOutcome::failure(PathBuf::new(), error)];
    }
    inputs.into_iter().map(|input| JobOutcome::failure(input, error.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_input_order_when_mixed_inputs_fail() {
        let inputs = vec![PathBuf::from("first.png"), PathBuf::from("second.txt"), PathBuf::from("third.png")];
        let outcomes = BatchRunner::run("convert_images", inputs.clone(), |input| JobOutcome {
            input_path: input,
            output_paths: vec![PathBuf::from("output")],
            detail: "processed".to_string(),
            failure: None,
        });

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs);
        assert!(outcomes[0].failure.is_none());
        assert!(outcomes[1].failure.is_some());
        assert!(outcomes[2].failure.is_none());
    }

    #[test]
    fn rejects_invalid_single_input_cardinality_before_running_jobs() {
        let ran = std::sync::atomic::AtomicBool::new(false);
        let outcomes = BatchRunner::run("generate_icon_set", vec![PathBuf::from("first.png"), PathBuf::from("second.png")], |_| {
            ran.store(true, std::sync::atomic::Ordering::Relaxed);
            JobOutcome::failure(PathBuf::new(), ToolError::processing("job should not run"))
        });

        assert!(!ran.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|outcome| outcome.failure.is_some()));
    }
}
