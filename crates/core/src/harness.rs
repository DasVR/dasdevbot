//! Provider-agnostic teammate runtime: wake, plan, act, approval, checkpoint, resume, finish.
//! Wake, plan, act, request approval, checkpoint, resume, finish.
//! A limit hit pauses on the last checkpoint. It does not fail the job.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Wake,
    Plan,
    Act,
    RequestApproval,
    Checkpoint,
    Finished,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Wake => "wake",
            Phase::Plan => "plan",
            Phase::Act => "act",
            Phase::RequestApproval => "request_approval",
            Phase::Checkpoint => "checkpoint",
            Phase::Finished => "finished",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "wake" => Some(Phase::Wake),
            "plan" => Some(Phase::Plan),
            "act" => Some(Phase::Act),
            "request_approval" => Some(Phase::RequestApproval),
            "checkpoint" => Some(Phase::Checkpoint),
            "finished" => Some(Phase::Finished),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Begin,
    Planned,
    Tool,
    NeedApproval,
    Asked,
    Checkpoint,
    Resume,
    Finish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobLife {
    Running,
    Paused,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarnessState {
    pub phase: Phase,
    pub paused: bool,
    pub step: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessError {
    Paused,
    Illegal,
}

impl HarnessState {
    pub fn wake() -> Self {
        Self {
            phase: Phase::Wake,
            paused: false,
            step: 0,
        }
    }

    pub fn life(&self) -> JobLife {
        if self.paused {
            JobLife::Paused
        } else if self.phase == Phase::Finished {
            JobLife::Finished
        } else {
            JobLife::Running
        }
    }

    /// Stay on the last phase. The caller checkpoints before calling this
    /// when the limit arrives mid-act.
    pub fn on_limit(mut self) -> Self {
        self.paused = true;
        self
    }

    pub fn resume(mut self) -> Self {
        self.paused = false;
        self
    }

    pub fn advance(mut self, step: Step) -> Result<Self, HarnessError> {
        if self.paused {
            return Err(HarnessError::Paused);
        }
        let next = match self.phase {
            Phase::Wake => match step {
                Step::Begin => Phase::Plan,
                Step::Planned
                | Step::Tool
                | Step::NeedApproval
                | Step::Asked
                | Step::Checkpoint
                | Step::Resume
                | Step::Finish => return Err(HarnessError::Illegal),
            },
            Phase::Plan => match step {
                Step::Planned => Phase::Act,
                Step::Begin
                | Step::Tool
                | Step::NeedApproval
                | Step::Asked
                | Step::Checkpoint
                | Step::Resume
                | Step::Finish => return Err(HarnessError::Illegal),
            },
            Phase::Act => match step {
                Step::Tool => Phase::Act,
                Step::NeedApproval => Phase::RequestApproval,
                Step::Checkpoint => Phase::Checkpoint,
                Step::Begin | Step::Planned | Step::Asked | Step::Resume | Step::Finish => {
                    return Err(HarnessError::Illegal)
                }
            },
            Phase::RequestApproval => match step {
                Step::Asked => Phase::Checkpoint,
                Step::Begin
                | Step::Planned
                | Step::Tool
                | Step::NeedApproval
                | Step::Checkpoint
                | Step::Resume
                | Step::Finish => return Err(HarnessError::Illegal),
            },
            Phase::Checkpoint => match step {
                Step::Resume => Phase::Act,
                Step::Finish => Phase::Finished,
                Step::Begin
                | Step::Planned
                | Step::Tool
                | Step::NeedApproval
                | Step::Asked
                | Step::Checkpoint => return Err(HarnessError::Illegal),
            },
            Phase::Finished => match step {
                Step::Begin
                | Step::Planned
                | Step::Tool
                | Step::NeedApproval
                | Step::Asked
                | Step::Checkpoint
                | Step::Resume
                | Step::Finish => return Err(HarnessError::Illegal),
            },
        };
        self.phase = next;
        self.step = self.step.saturating_add(1);
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_is_wake_plan_act_approval_checkpoint_resume_finish() {
        let state = HarnessState::wake()
            .advance(Step::Begin)
            .unwrap()
            .advance(Step::Planned)
            .unwrap()
            .advance(Step::Tool)
            .unwrap()
            .advance(Step::NeedApproval)
            .unwrap()
            .advance(Step::Asked)
            .unwrap();
        assert_eq!(state.phase, Phase::Checkpoint);
        let resumed = state.advance(Step::Resume).unwrap();
        assert_eq!(resumed.phase, Phase::Act);
        let done = resumed
            .advance(Step::Checkpoint)
            .unwrap()
            .advance(Step::Finish)
            .unwrap();
        assert_eq!(done.phase, Phase::Finished);
        assert_eq!(done.life(), JobLife::Finished);
    }

    #[test]
    fn a_limit_pauses_at_the_checkpoint_instead_of_failing() {
        let checkpoint = HarnessState::wake()
            .advance(Step::Begin)
            .unwrap()
            .advance(Step::Planned)
            .unwrap()
            .advance(Step::Checkpoint)
            .unwrap();
        let step = checkpoint.step;
        let paused = checkpoint.on_limit();
        assert!(paused.paused);
        assert_eq!(paused.phase, Phase::Checkpoint);
        assert_eq!(paused.step, step);
        assert_eq!(paused.life(), JobLife::Paused);
        assert!(paused.advance(Step::Finish).is_err());
        let resumed = paused.resume();
        assert_eq!(resumed.life(), JobLife::Running);
        assert_eq!(
            resumed.advance(Step::Finish).unwrap().phase,
            Phase::Finished
        );
    }
}
