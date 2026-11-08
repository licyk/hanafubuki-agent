use super::*;

pub trait AgentExecutionProcessPort: Send + Sync {
    fn reserve(
        &self,
        process_id: &str,
        owner: &ProcessOwnerId,
    ) -> Result<ProcessReservation, String>;
    fn finalize_reservation(&self, reservation: &ProcessReservation) -> OwnedReleaseStatus;
    fn start_reserved(
        &self,
        request: ExecutorStartRequest,
        reservation: &ProcessReservation,
    ) -> Result<(ExecutorInfo, ProcessReceipt), String>;
    fn info(&self, receipt: &ProcessReceipt) -> Result<ExecutorInfo, String>;
    fn read(
        &self,
        receipt: &ProcessReceipt,
        from_offset: Option<u64>,
        max_bytes: Option<usize>,
    ) -> Result<ExecutorOutput, String>;
    fn terminate(&self, receipt: &ProcessReceipt) -> Result<(), String>;
    fn dispose_confirmed(&self, receipt: &ProcessReceipt) -> Result<(), String>;
    fn write(&self, receipt: &ProcessReceipt, bytes: Vec<u8>, close: bool) -> Result<(), String>;
    fn interrupt(&self, receipt: &ProcessReceipt) -> Result<(), String>;
    fn resize(&self, receipt: &ProcessReceipt, cols: u16, rows: u16) -> Result<(), String>;
}

#[derive(Clone)]
pub struct ExecutorAgentExecutionPort {
    executor: ExecutorManager,
}

impl ExecutorAgentExecutionPort {
    pub fn new(executor: ExecutorManager) -> Self {
        Self { executor }
    }
}

impl AgentExecutionProcessPort for ExecutorAgentExecutionPort {
    fn reserve(
        &self,
        process_id: &str,
        owner: &ProcessOwnerId,
    ) -> Result<ProcessReservation, String> {
        reserve_executor_id(&self.executor, process_id, owner)
    }

    fn finalize_reservation(&self, reservation: &ProcessReservation) -> OwnedReleaseStatus {
        finalize_executor_reservation_handle(&self.executor, reservation)
    }

    fn start_reserved(
        &self,
        request: ExecutorStartRequest,
        reservation: &ProcessReservation,
    ) -> Result<(ExecutorInfo, ProcessReceipt), String> {
        start_reserved_executor(&self.executor, request, reservation)
    }

    fn info(&self, receipt: &ProcessReceipt) -> Result<ExecutorInfo, String> {
        get_reserved_executor_info(&self.executor, receipt)
    }

    fn read(
        &self,
        receipt: &ProcessReceipt,
        from_offset: Option<u64>,
        max_bytes: Option<usize>,
    ) -> Result<ExecutorOutput, String> {
        read_reserved_executor_output(&self.executor, receipt, from_offset, max_bytes)
    }

    fn terminate(&self, receipt: &ProcessReceipt) -> Result<(), String> {
        terminate_reserved_executor(&self.executor, receipt)
    }

    fn dispose_confirmed(&self, receipt: &ProcessReceipt) -> Result<(), String> {
        dispose_reserved_executor(&self.executor, receipt)
    }

    fn write(&self, receipt: &ProcessReceipt, bytes: Vec<u8>, close: bool) -> Result<(), String> {
        write_reserved_executor_stdin(&self.executor, receipt, bytes, close)
    }

    fn interrupt(&self, receipt: &ProcessReceipt) -> Result<(), String> {
        interrupt_reserved_executor(&self.executor, receipt)
    }

    fn resize(&self, receipt: &ProcessReceipt, cols: u16, rows: u16) -> Result<(), String> {
        resize_reserved_executor_pty(&self.executor, receipt, cols, rows)
    }
}

pub(super) fn executor_request(
    process_id: String,
    plan: ResolvedCommandPlan,
) -> ExecutorStartRequest {
    ExecutorStartRequest {
        executor_id: Some(process_id),
        command: plan.command,
        cwd: Some(plan.cwd),
        environment_inheritance: EnvironmentInheritance::Clear,
        env: plan
            .environment
            .into_iter()
            .map(|(key, value)| (key, Some(value)))
            .collect(),
        io_mode: plan.io_mode,
        stream_stdin: plan.interactive,
        stream_output: true,
        cols: plan.cols,
        rows: plan.rows,
        timeout_ms: Some(plan.timeout_ms),
        output_bytes_cap: Some(plan.output_bytes_cap),
    }
}
