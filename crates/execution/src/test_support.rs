use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use hanafubuki_agent_process::executor::{
    ExecutorInfo, ExecutorIoMode, ExecutorOutput, ExecutorStartRequest, ExecutorStatus,
    OutputDelta, OutputStream,
};
use hanafubuki_agent_process::ownership::{
    OwnedReleaseStatus, ProcessOwnerId, ProcessReceipt, ProcessReservation,
};

use crate::AgentExecutionProcessPort;

#[derive(Default)]
pub struct FakeAgentExecutionProcessPort {
    state: Mutex<FakeState>,
}

#[derive(Default)]
struct FakeState {
    sessions: HashMap<String, FakeSession>,
    reservations: HashMap<String, ProcessOwnerId>,
    dispose_failures: usize,
    terminate_failures: usize,
    starts: Vec<ExecutorStartRequest>,
    writes: Vec<Vec<u8>>,
    interrupts: usize,
    write_delay_ms: u64,
    finish_on_start: Option<(Vec<u8>, i32)>,
}

struct FakeSession {
    receipt: ProcessReceipt,
    status: ExecutorStatus,
    exit_code: Option<i32>,
    output: Vec<u8>,
    io_mode: ExecutorIoMode,
}

impl FakeAgentExecutionProcessPort {
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn fail_next_disposals(&self, count: usize) {
        self.state.lock().unwrap().dispose_failures = count;
    }

    pub fn fail_next_terminations(&self, count: usize) {
        self.state.lock().unwrap().terminate_failures = count;
    }

    pub fn finish(&self, output: impl Into<Vec<u8>>, exit_code: i32) {
        let mut state = self.state.lock().unwrap();
        let session = state.sessions.values_mut().next().expect("fake session");
        session.output = output.into();
        session.exit_code = Some(exit_code);
        session.status = ExecutorStatus::Exited;
    }

    pub fn time_out(&self) {
        let mut state = self.state.lock().unwrap();
        let session = state.sessions.values_mut().next().expect("fake session");
        session.exit_code = None;
        session.status = ExecutorStatus::TimedOut;
    }

    pub fn starts(&self) -> Vec<ExecutorStartRequest> {
        self.state.lock().unwrap().starts.clone()
    }

    pub fn session_count(&self) -> usize {
        self.state.lock().unwrap().sessions.len()
    }

    pub fn reservation_count(&self) -> usize {
        self.state.lock().unwrap().reservations.len()
    }

    pub fn writes(&self) -> Vec<Vec<u8>> {
        self.state.lock().unwrap().writes.clone()
    }

    pub fn interrupt_count(&self) -> usize {
        self.state.lock().unwrap().interrupts
    }
    pub fn set_write_delay_ms(&self, value: u64) {
        self.state.lock().unwrap().write_delay_ms = value;
    }

    pub fn finish_on_start(&self, output: impl Into<Vec<u8>>, exit_code: i32) {
        self.state.lock().unwrap().finish_on_start = Some((output.into(), exit_code));
    }

    pub fn set_output(&self, process_id: &str, output: Vec<u8>) {
        self.state
            .lock()
            .unwrap()
            .sessions
            .get_mut(process_id)
            .expect("fake session")
            .output = output;
    }
}

impl AgentExecutionProcessPort for FakeAgentExecutionProcessPort {
    fn reserve(
        &self,
        process_id: &str,
        owner: &ProcessOwnerId,
    ) -> Result<ProcessReservation, String> {
        let mut state = self.state.lock().unwrap();
        if state.sessions.contains_key(process_id) || state.reservations.contains_key(process_id) {
            return Err("already reserved".to_string());
        }
        state
            .reservations
            .insert(process_id.to_string(), owner.clone());
        Ok(ProcessReservation::for_test(process_id, owner.clone()))
    }

    fn finalize_reservation(&self, reservation: &ProcessReservation) -> OwnedReleaseStatus {
        let mut state = self.state.lock().unwrap();
        match state.reservations.get(reservation.process_id()) {
            Some(owner) if owner == reservation.owner_id() => {
                state.reservations.remove(reservation.process_id());
                OwnedReleaseStatus::Released
            }
            Some(_) => OwnedReleaseStatus::DifferentOwner,
            None => OwnedReleaseStatus::Missing,
        }
    }

    fn start_reserved(
        &self,
        request: ExecutorStartRequest,
        reservation: &ProcessReservation,
    ) -> Result<(ExecutorInfo, ProcessReceipt), String> {
        let mut state = self.state.lock().unwrap();
        let owner = state
            .reservations
            .remove(reservation.process_id())
            .ok_or_else(|| "missing reservation".to_string())?;
        let receipt = ProcessReceipt::for_test(reservation.process_id(), owner);
        let io_mode = request.io_mode;
        let info = ExecutorInfo {
            executor_id: reservation.process_id().to_string(),
            status: ExecutorStatus::Running,
            io_mode,
            command: request.command.clone(),
            exit_code: None,
            error: None,
            output_offset: 0,
        };
        state.starts.push(request);
        let finish_on_start = state.finish_on_start.take();
        state.sessions.insert(
            reservation.process_id().to_string(),
            FakeSession {
                receipt: receipt.clone(),
                status: finish_on_start
                    .as_ref()
                    .map(|_| ExecutorStatus::Exited)
                    .unwrap_or(ExecutorStatus::Running),
                exit_code: finish_on_start.as_ref().map(|(_, code)| *code),
                output: finish_on_start
                    .map(|(output, _)| output)
                    .unwrap_or_default(),
                io_mode,
            },
        );
        Ok((info, receipt))
    }

    fn info(&self, receipt: &ProcessReceipt) -> Result<ExecutorInfo, String> {
        let state = self.state.lock().unwrap();
        let session = exact_session(&state, receipt)?;
        Ok(ExecutorInfo {
            executor_id: receipt.process_id().to_string(),
            status: session.status,
            io_mode: session.io_mode,
            command: Vec::new(),
            exit_code: session.exit_code,
            error: None,
            output_offset: session.output.len() as u64,
        })
    }

    fn read(
        &self,
        receipt: &ProcessReceipt,
        from_offset: Option<u64>,
        max_bytes: Option<usize>,
    ) -> Result<ExecutorOutput, String> {
        let state = self.state.lock().unwrap();
        let session = exact_session(&state, receipt)?;
        let requested = from_offset.unwrap_or(0) as usize;
        let oldest = session
            .output
            .len()
            .saturating_sub(max_bytes.unwrap_or(usize::MAX));
        let start = requested.max(oldest).min(session.output.len());
        let end = start
            .saturating_add(max_bytes.unwrap_or(usize::MAX))
            .min(session.output.len());
        Ok(ExecutorOutput {
            executor_id: receipt.process_id().to_string(),
            status: session.status,
            exit_code: session.exit_code,
            chunks: if start == end {
                Vec::new()
            } else {
                vec![OutputDelta {
                    stream: if session.io_mode.uses_pty() {
                        OutputStream::Merged
                    } else {
                        OutputStream::Stdout
                    },
                    data: session.output[start..end].to_vec(),
                }]
            },
            start_offset: start as u64,
            next_offset: end as u64,
            truncated: requested < oldest,
            terminal_size: None,
            resizes: Vec::new(),
        })
    }

    fn terminate(&self, receipt: &ProcessReceipt) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        if state.terminate_failures > 0 {
            state.terminate_failures -= 1;
            return Err("injected termination failure".to_string());
        }
        let session = exact_session_mut(&mut state, receipt)?;
        session.status = ExecutorStatus::Cancelled;
        Ok(())
    }

    fn dispose_confirmed(&self, receipt: &ProcessReceipt) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        if state.dispose_failures > 0 {
            state.dispose_failures -= 1;
            return Err("injected disposal failure".to_string());
        }
        exact_session(&state, receipt)?;
        state.sessions.remove(receipt.process_id());
        Ok(())
    }

    fn write(&self, receipt: &ProcessReceipt, bytes: Vec<u8>, _close: bool) -> Result<(), String> {
        let delay = {
            let state = self.state.lock().unwrap();
            exact_session(&state, receipt)?;
            state.write_delay_ms
        };
        if delay > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay));
        }
        let mut state = self.state.lock().unwrap();
        exact_session(&state, receipt)?;
        state.writes.push(bytes);
        Ok(())
    }

    fn interrupt(&self, receipt: &ProcessReceipt) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        exact_session(&state, receipt)?;
        state.interrupts += 1;
        Ok(())
    }

    fn resize(&self, receipt: &ProcessReceipt, _cols: u16, _rows: u16) -> Result<(), String> {
        let state = self.state.lock().unwrap();
        exact_session(&state, receipt)?;
        Ok(())
    }
}

fn exact_session<'a>(
    state: &'a FakeState,
    receipt: &ProcessReceipt,
) -> Result<&'a FakeSession, String> {
    state
        .sessions
        .get(receipt.process_id())
        .filter(|session| &session.receipt == receipt)
        .ok_or_else(|| "stale fake receipt".to_string())
}

fn exact_session_mut<'a>(
    state: &'a mut FakeState,
    receipt: &ProcessReceipt,
) -> Result<&'a mut FakeSession, String> {
    state
        .sessions
        .get_mut(receipt.process_id())
        .filter(|session| &session.receipt == receipt)
        .ok_or_else(|| "stale fake receipt".to_string())
}
