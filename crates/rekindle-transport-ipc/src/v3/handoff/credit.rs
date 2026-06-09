//! SideChannel credit tracking — bounds kernel SOCK_SEQPACKET buffer occupancy.

#[derive(Debug)]
pub enum CreditError {
    Exhausted,
}

pub struct SideChannelCreditTracker {
    remaining: u16,
    generation: u32,
}

impl SideChannelCreditTracker {
    pub fn new(initial_credit: u16) -> Self {
        Self { remaining: initial_credit, generation: 0 }
    }

    pub fn try_consume(&mut self) -> Result<(), CreditError> {
        if self.remaining == 0 {
            return Err(CreditError::Exhausted);
        }
        self.remaining -= 1;
        Ok(())
    }

    pub fn replenish(&mut self, additional: u16, generation: u32) {
        if generation <= self.generation {
            return;
        }
        self.generation = generation;
        self.remaining = additional;
    }

    pub fn remaining(&self) -> u16 {
        self.remaining
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }
}
