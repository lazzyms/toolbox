//! A model of how Toolbox reserves and publishes an output file, checked with Stateright.
//!
//! Each writer action is one filesystem call in `OutputNaming::reserve_candidate`,
//! `OutputReservation::publish` or `Drop for OutputReservation` in
//! `src-tauri/src/kit/common/mod.rs`. Change the model in the same commit as that code.
//!
//! Two Toolbox writers race for the same two candidate names. A third party (the user,
//! another app) ignores the claim files and saves into an empty name at any moment.

use stateright::{Model, Property};

pub const NAMES: usize = 2;
pub const WRITERS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Protocol {
    /// Today's protocol. The job writes a private temporary file, publishing hard-links it
    /// to the destination, which fails if a file is already there, and dropping the
    /// reservation removes only the temporary file and the claim.
    Current,
    /// The protocol from f4d7116e. The job wrote the destination itself, and dropping an
    /// unpublished reservation deleted the destination.
    DeletesDestinationOnDrop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Owner {
    Writer(usize),
    ThirdParty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Step {
    CheckExists(usize),
    Claim(usize),
    RecheckAfterClaim(usize),
    CreateTemporary(usize),
    RecheckAfterTemporary(usize),
    Reserved(usize),
    Written(usize),
    Dropping(usize),
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct State {
    pub destination: [Option<Owner>; NAMES],
    pub claim: [bool; NAMES],
    pub temporary: [Option<usize>; WRITERS],
    pub steps: [Step; WRITERS],
    pub published: [Option<usize>; WRITERS],
    pub third_party_saved: [bool; NAMES],
    pub publish_hit_other_writer: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Writer(usize, Move),
    ThirdPartySaves(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Move {
    Advance,
    JobFails,
    Publish,
}

pub struct Reservation {
    pub protocol: Protocol,
}

fn next_candidate(name: usize) -> Step {
    if name + 1 < NAMES { Step::CheckExists(name + 1) } else { Step::Done }
}

impl Model for Reservation {
    type State = State;
    type Action = Action;

    fn init_states(&self) -> Vec<State> {
        vec![State {
            destination: [None; NAMES],
            claim: [false; NAMES],
            temporary: [None; WRITERS],
            steps: [Step::CheckExists(0); WRITERS],
            published: [None; WRITERS],
            third_party_saved: [false; NAMES],
            publish_hit_other_writer: false,
        }]
    }

    fn actions(&self, state: &State, actions: &mut Vec<Action>) {
        for (writer, step) in state.steps.iter().enumerate() {
            match step {
                Step::Done => {}
                Step::Reserved(_) if self.protocol == Protocol::Current => {
                    actions.push(Action::Writer(writer, Move::JobFails));
                    actions.push(Action::Writer(writer, Move::Publish));
                }
                Step::Reserved(_) => {
                    actions.push(Action::Writer(writer, Move::JobFails));
                    actions.push(Action::Writer(writer, Move::Advance));
                }
                Step::Written(_) => {
                    actions.push(Action::Writer(writer, Move::JobFails));
                    actions.push(Action::Writer(writer, Move::Publish));
                }
                _ => actions.push(Action::Writer(writer, Move::Advance)),
            }
        }
        for name in 0..NAMES {
            if state.destination[name].is_none() && !state.third_party_saved.iter().any(|saved| *saved) {
                actions.push(Action::ThirdPartySaves(name));
            }
        }
    }

    fn next_state(&self, state: &State, action: Action) -> Option<State> {
        let mut next = state.clone();
        match action {
            Action::ThirdPartySaves(name) => {
                next.destination[name] = Some(Owner::ThirdParty);
                next.third_party_saved[name] = true;
            }
            Action::Writer(writer, choice) => {
                next.steps[writer] = match (state.steps[writer], choice) {
                    (Step::CheckExists(name), _) if state.destination[name].is_some() => next_candidate(name),
                    (Step::CheckExists(name), _) => Step::Claim(name),
                    (Step::Claim(name), _) if state.claim[name] => next_candidate(name),
                    (Step::Claim(name), _) => {
                        next.claim[name] = true;
                        Step::RecheckAfterClaim(name)
                    }
                    (Step::RecheckAfterClaim(name), _) if state.destination[name].is_some() => {
                        next.claim[name] = false;
                        next_candidate(name)
                    }
                    (Step::RecheckAfterClaim(name), _) => match self.protocol {
                        Protocol::Current => Step::CreateTemporary(name),
                        Protocol::DeletesDestinationOnDrop => Step::Reserved(name),
                    },
                    (Step::CreateTemporary(name), _) => {
                        next.temporary[writer] = Some(name);
                        Step::RecheckAfterTemporary(name)
                    }
                    (Step::RecheckAfterTemporary(name), _) if state.destination[name].is_some() => {
                        next.temporary[writer] = None;
                        next.claim[name] = false;
                        next_candidate(name)
                    }
                    (Step::RecheckAfterTemporary(name), _) => Step::Reserved(name),
                    (Step::Reserved(name), Move::Publish) => {
                        match state.destination[name] {
                            None => {
                                next.destination[name] = Some(Owner::Writer(writer));
                                next.published[writer] = Some(name);
                            }
                            Some(Owner::Writer(_)) => next.publish_hit_other_writer = true,
                            Some(Owner::ThirdParty) => {}
                        }
                        Step::Dropping(name)
                    }
                    (Step::Reserved(name), Move::Advance) => {
                        next.destination[name] = Some(Owner::Writer(writer));
                        Step::Written(name)
                    }
                    (Step::Written(name), Move::Publish) => {
                        next.published[writer] = Some(name);
                        next.claim[name] = false;
                        Step::Done
                    }
                    (Step::Reserved(name) | Step::Written(name), Move::JobFails) => Step::Dropping(name),
                    (Step::Dropping(name), _) => {
                        if self.protocol == Protocol::DeletesDestinationOnDrop {
                            next.destination[name] = None;
                        }
                        next.temporary[writer] = None;
                        next.claim[name] = false;
                        Step::Done
                    }
                    (step, _) => unreachable!("{step:?} has no move {choice:?}"),
                };
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("a third party's file is never deleted or replaced", |_, state: &State| {
                (0..NAMES).all(|name| !state.third_party_saved[name] || state.destination[name] == Some(Owner::ThirdParty))
            }),
            Property::always("a published output is never deleted or replaced", |_, state: &State| {
                (0..WRITERS).all(|writer| match state.published[writer] {
                    Some(name) => state.destination[name] == Some(Owner::Writer(writer)),
                    None => true,
                })
            }),
            Property::always("two Toolbox writers never pick the same name", |_, state: &State| !state.publish_hit_other_writer),
            Property::always("finished writers leave no claim or temporary file", |_, state: &State| {
                !state.steps.iter().all(|step| *step == Step::Done)
                    || (!state.claim.iter().any(|claim| *claim) && state.temporary.iter().all(Option::is_none))
            }),
            Property::sometimes("both writers publish", |_, state: &State| state.published.iter().all(Option::is_some)),
            Property::sometimes("a writer publishes beside a third party's file", |_, state: &State| {
                state.third_party_saved.iter().any(|saved| *saved) && state.published.iter().any(Option::is_some)
            }),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stateright::Checker;

    #[test]
    fn current_protocol_never_loses_a_file_or_leaves_debris() {
        let checker = Reservation { protocol: Protocol::Current }.checker().spawn_bfs().join();
        checker.assert_properties();
    }

    #[test]
    fn the_f4d7116e_protocol_deletes_a_third_partys_file() {
        let checker = Reservation { protocol: Protocol::DeletesDestinationOnDrop }.checker().spawn_bfs().join();
        let trace = checker.assert_any_discovery("a third party's file is never deleted or replaced");
        println!("{}", trace.into_actions().iter().map(|action| format!("{action:?}")).collect::<Vec<_>>().join("\n"));
    }
}
