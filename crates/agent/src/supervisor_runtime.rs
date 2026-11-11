/// The own runtime of a worker in a contained group (the memory
/// supervisor, the model discovery), driven by the worker's thread. Its
/// blocking pool (a resolver's lookups) is contained too.
pub(super) fn build_agent_supervisor_runtime() -> Result<tokio::runtime::Runtime, std::io::Error> {
    hanafubuki_agent_runtime::contained_threads(&mut tokio::runtime::Builder::new_current_thread())
        .enable_all()
        .build()
}

#[cfg(test)]
mod tests;
