//! A small driver: agents on one `stochcraft_des::EventQueue`.

use stochcraft_des::EventQueue;

use crate::interp::{Agent, RunError};

/// Runs agents on one discrete-event clock (ROADMAP D18). Every event an agent sends itself
/// with a delay, or with none, is an event on the queue at the time it is due; popped, it is
/// handled by one complete macrostep of its agent, in zero simulated time. Events due at the
/// same time pop in the order they were sent. A cancelled or redrawn send stays on the queue
/// and is skipped when it is popped.
pub struct Driver {
    queue: EventQueue<(u32, u64)>,
    agents: Vec<Agent>,
}

impl std::fmt::Debug for Driver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Driver")
            .field("now", &self.queue.now())
            .field("queued", &self.queue.len())
            .field("agents", &self.agents)
            .finish()
    }
}

impl Default for Driver {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl Driver {
    /// A driver whose clock starts at `start`.
    pub fn new(start: f64) -> Self {
        Self {
            queue: EventQueue::new(start),
            agents: Vec::new(),
        }
    }

    /// The current time: the time of the last event handled.
    pub fn now(&self) -> f64 {
        self.queue.now()
    }

    /// Adds an agent and starts it now (its first macrostep). Returns its number. Agents start
    /// in the order they are added.
    pub fn add(&mut self, agent: Agent) -> Result<u32, RunError> {
        let i = self.agents.len() as u32;
        self.agents.push(agent);
        let now = self.queue.now();
        if let Some(a) = self.agents.get_mut(i as usize) {
            a.start(now)?;
        }
        self.flush(i)?;
        Ok(i)
    }

    pub fn agent(&self, i: u32) -> Option<&Agent> {
        self.agents.get(i as usize)
    }

    pub fn agents(&self) -> &[Agent] {
        &self.agents
    }

    /// Sends agent `i` the event `name`, `delay` from now. Returns false if there is no such
    /// agent, it has ended, or the delay is not a time of 0 or more.
    pub fn post(&mut self, i: u32, name: &str, delay: f64) -> Result<bool, RunError> {
        let Some(a) = self.agents.get_mut(i as usize) else {
            return Ok(false);
        };
        if a.post(name, delay).is_none() {
            return Ok(false);
        }
        self.flush(i)?;
        Ok(true)
    }

    /// Handles the next event that is due no later than `end`. Returns its time, or `None` if
    /// there is none.
    pub fn step_until(&mut self, end: f64) -> Result<Option<f64>, RunError> {
        while self.queue.peek_time().is_some_and(|t| t <= end) {
            let Some(ev) = self.queue.pop() else {
                break;
            };
            let (i, token) = ev.payload;
            let Some(agent) = self.agents.get_mut(i as usize) else {
                continue;
            };
            if agent.fire(token, ev.time)? {
                self.flush(i)?;
                return Ok(Some(ev.time));
            }
        }
        Ok(None)
    }

    /// Handles the next event. Returns its time, or `None` if no event is pending.
    pub fn step(&mut self) -> Result<Option<f64>, RunError> {
        self.step_until(f64::INFINITY)
    }

    /// Handles every event due no later than `end`, then moves the clock to `end`. Returns how
    /// many events were handled.
    pub fn run_until(&mut self, end: f64) -> Result<u64, RunError> {
        let mut handled = 0;
        while self.step_until(end)?.is_some() {
            handled += 1;
        }
        self.queue.advance_to(end);
        Ok(handled)
    }

    /// Puts agent `i`'s new sends on the queue.
    fn flush(&mut self, i: u32) -> Result<(), RunError> {
        let Some(agent) = self.agents.get_mut(i as usize) else {
            return Ok(());
        };
        let now = self.queue.now();
        for s in agent.take_sends() {
            if self.queue.schedule_in(s.delay, 0, (i, s.token)).is_err() {
                // The agent stops: its pending sends are dropped, so those already queued are
                // skipped when popped.
                agent.halt(format!(
                    "an event sent {} after time {now} would not be at a finite time",
                    s.delay
                ));
                return Err(RunError::Schedule {
                    agent: agent.key().to_string(),
                    time: now,
                    delay: s.delay,
                });
            }
        }
        Ok(())
    }
}
