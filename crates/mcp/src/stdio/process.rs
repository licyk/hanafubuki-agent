use super::*;

pub(super) struct McpStdioProcess {
    manager: Arc<ExecutorManager>,
    workers: BackgroundWorkerGroup,
    receipt: ReservedExecutorReceipt,
    diagnostic: Arc<Mutex<String>>,
    exit_reason: Arc<Mutex<Option<String>>>,
    resolved_program: crate::McpResolvedProgram,
}

impl McpStdioProcess {
    pub(super) async fn spawn(
        server: &McpServerConfig,
        config: &McpStdioConfig,
        secrets: McpResolvedSecretEnvironment,
        manager: Arc<ExecutorManager>,
        workers: BackgroundWorkerGroup,
    ) -> Result<(Arc<Self>, McpExecutorTransport), McpError> {
        let mut environment = config
            .public_environment
            .iter()
            .map(|(name, value)| (name.clone(), Some(value.clone())))
            .collect::<HashMap<_, _>>();
        for (name, value) in secrets.0 {
            if environment.insert(name, Some(value)).is_some() {
                return Err(McpError::InvalidConfig(
                    "public and secret environment entries overlap".into(),
                ));
            }
        }
        let inherited_path = std::env::var_os("PATH");
        let configured_path = environment.iter().find_map(|(name, value)| {
            name.eq_ignore_ascii_case("PATH")
                .then(|| value.as_deref().map(std::ffi::OsStr::new))
                .flatten()
        });
        let resolved_program = crate::resolve_stdio_program(
            &config.program,
            configured_path.or(inherited_path.as_deref()),
        )?;
        let executor_id = format!(
            "mcp-{}-{}",
            server.id.as_str(),
            NEXT_MCP_EXECUTOR.fetch_add(1, Ordering::Relaxed)
        );
        let owner = ProcessOwnerId::generate();
        let reservation = reserve_executor_id(&manager, &executor_id, &owner)
            .map_err(|error| McpError::Unavailable(bounded_error(error)))?;
        let mut command = Vec::with_capacity(config.argv.len() + 1);
        command.push(resolved_program.resolved_path.to_string_lossy().to_string());
        command.extend(config.argv.clone());
        let request = ExecutorStartRequest {
            executor_id: Some(executor_id),
            command,
            cwd: config.cwd.clone(),
            environment_inheritance: EnvironmentInheritance::Inherit,
            env: environment,
            io_mode: ExecutorIoMode::Pipe,
            stream_stdin: true,
            stream_output: true,
            cols: None,
            rows: None,
            timeout_ms: None,
            output_bytes_cap: Some(MCP_EXECUTOR_OUTPUT_CAP),
        };
        let manager_for_spawn = manager.clone();
        let started = on_worker(
            &workers,
            format!("mcp-stdio-spawn-{}", server.id.as_str()),
            move || {
                start_reserved_executor(&manager_for_spawn, request, &reservation)
                    .map_err(|error| (error, reservation))
            },
        )
        .await
        .map_err(|error| McpError::Unavailable(bounded_error(error)))?;
        let (_, receipt) = match started {
            Ok(started) => started,
            Err((error, reservation)) => {
                release_executor_reservation(&manager, &reservation);
                return Err(McpError::Unavailable(bounded_error(error)));
            }
        };

        let diagnostic = Arc::new(Mutex::new(String::new()));
        let exit_reason = Arc::new(Mutex::new(None));
        let (commands, command_rx) = mpsc::sync_channel(MCP_TRANSPORT_CHANNEL_CAPACITY);
        let (inbound_tx, inbound) = tokio_mpsc::channel(MCP_TRANSPORT_CHANNEL_CAPACITY);
        let worker_manager = manager.clone();
        let worker_receipt = receipt.clone();
        let worker_diagnostic = diagnostic.clone();
        let worker_exit_reason = exit_reason.clone();
        let worker_group = workers.clone();
        if let Err(error) = worker_group.spawn(
            format!("mcp-stdio-{}", server.id.as_str()),
            move |cancellation| {
                run_stdio_worker(
                    worker_manager,
                    worker_receipt,
                    command_rx,
                    inbound_tx,
                    worker_diagnostic,
                    worker_exit_reason,
                    cancellation,
                )
            },
        ) {
            let cleanup_manager = manager.clone();
            let cleanup_receipt = receipt.clone();
            let cleanup = on_worker(&workers, "mcp-stdio-cleanup".to_string(), move || {
                dispose_reserved_executor(&cleanup_manager, &cleanup_receipt)
            })
            .await
            .map_err(|error| McpError::CleanupUnconfirmed(bounded_error(error)))?;
            cleanup.map_err(|cleanup| McpError::CleanupUnconfirmed(bounded_error(cleanup)))?;
            return Err(McpError::Unavailable(bounded_error(error)));
        }
        Ok((
            Arc::new(Self {
                manager,
                workers,
                receipt,
                diagnostic,
                exit_reason,
                resolved_program,
            }),
            McpExecutorTransport {
                commands,
                inbound,
                closed: false,
            },
        ))
    }

    pub(super) async fn dispose_confirmed(&self) -> Result<(), McpError> {
        let manager = self.manager.clone();
        let receipt = self.receipt.clone();
        on_worker(&self.workers, "mcp-stdio-dispose".to_string(), move || {
            dispose_reserved_executor(&manager, &receipt)
        })
        .await
        .map_err(|error| McpError::CleanupUnconfirmed(bounded_error(error)))?
        .map_err(|error| McpError::CleanupUnconfirmed(bounded_error(error)))
    }

    pub(super) fn diagnostic(&self) -> String {
        self.diagnostic.lock().clone()
    }

    /// Why the server's side of the transport ended, once it has.
    pub(super) fn exit_reason(&self) -> Option<String> {
        self.exit_reason.lock().clone()
    }

    pub(super) fn resolved_program(&self) -> crate::McpResolvedProgram {
        self.resolved_program.clone()
    }
}

/// Run blocking executor work on the server's worker group and await its
/// result, so the async caller never parks a runtime thread on the executor.
async fn on_worker<T: Send + 'static>(
    workers: &BackgroundWorkerGroup,
    name: String,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    workers.spawn(name, move |_| {
        let _ = sender.send(work());
    })?;
    receiver
        .await
        .map_err(|_| "MCP stdio worker ended without a result".to_string())
}
