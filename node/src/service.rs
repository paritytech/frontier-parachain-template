//! Service and ServiceFactory implementation. Specialized wrapper over substrate service.

// std
use std::{path::Path, sync::Arc, time::Duration};

use cumulus_client_cli::CollatorOptions;
// Local Runtime Types
use frontier_parachain_runtime::{opaque::Block, RuntimeApi, TransactionConverter};

// Cumulus Imports
use cumulus_client_consensus_aura::{AuraConsensus, BuildAuraConsensusParams, SlotProportion};
use cumulus_client_consensus_common::{
	ParachainBlockImport as TParachainBlockImport, ParachainConsensus,
};
use cumulus_client_service::{
	build_network, build_relay_chain_interface, prepare_node_config, start_collator,
	start_full_node, BuildNetworkParams, StartCollatorParams, StartFullNodeParams,
};
use cumulus_primitives_core::ParaId;
use cumulus_relay_chain_interface::RelayChainInterface;

// Substrate Imports
use frame_benchmarking_cli::SUBSTRATE_REFERENCE_HARDWARE;
use futures::FutureExt;
use sc_client_api::Backend;
use sc_consensus::ImportQueue;
use sc_executor::{
	HeapAllocStrategy, NativeElseWasmExecutor, WasmExecutor, DEFAULT_HEAP_ALLOC_STRATEGY,
};
use sc_network::NetworkBlock;
use sc_network_sync::SyncingService;
use sc_service::{Configuration, PartialComponents, TFullBackend, TFullClient, TaskManager};
use sc_telemetry::{Telemetry, TelemetryHandle, TelemetryWorker, TelemetryWorkerHandle};
use sc_transaction_pool_api::OffchainTransactionPoolFactory;
use sp_core::U256;
use sp_keystore::KeystorePtr;
use substrate_prometheus_endpoint::Registry;

// Frontier
use crate::eth::{
	db_config_dir, new_frontier_partial, spawn_frontier_tasks, BackendType, EthConfiguration,
	FrontierBackend, FrontierBlockImport as TFrontierBlockImport, FrontierPartialComponents,
};

/// Native executor type.
pub struct ParachainNativeExecutor;

impl sc_executor::NativeExecutionDispatch for ParachainNativeExecutor {
	type ExtendHostFunctions = frame_benchmarking::benchmarking::HostFunctions;

	fn dispatch(method: &str, data: &[u8]) -> Option<Vec<u8>> {
		frontier_parachain_runtime::api::dispatch(method, data)
	}

	fn native_version() -> sc_executor::NativeVersion {
		frontier_parachain_runtime::native_version()
	}
}

type ParachainExecutor = NativeElseWasmExecutor<ParachainNativeExecutor>;

type ParachainClient = TFullClient<Block, RuntimeApi, ParachainExecutor>;

type ParachainBackend = TFullBackend<Block>;

type ParachainBlockImport = TParachainBlockImport<Block, FrontierBlockImport, ParachainBackend>;

type FrontierBlockImport = TFrontierBlockImport<Block, Arc<ParachainClient>, ParachainClient>;

/// Starts a `ServiceBuilder` for a full service.
///
/// Use this macro if you don't actually need the full service, but just the builder in order to
/// be able to perform chain operations.
#[allow(clippy::type_complexity)]
pub fn new_partial(
	config: &Configuration,
	eth_config: &EthConfiguration,
) -> Result<
	PartialComponents<
		ParachainClient,
		ParachainBackend,
		(),
		sc_consensus::DefaultImportQueue<Block, ParachainClient>,
		sc_transaction_pool::FullPool<Block, ParachainClient>,
		(
			ParachainBlockImport,
			Option<Telemetry>,
			Option<TelemetryWorkerHandle>,
			FrontierBackend,
			Arc<fc_rpc::OverrideHandle<Block>>,
		),
	>,
	sc_service::Error,
> {
	let telemetry = config
		.telemetry_endpoints
		.clone()
		.filter(|x| !x.is_empty())
		.map(|endpoints| -> Result<_, sc_telemetry::Error> {
			let worker = TelemetryWorker::new(16)?;
			let telemetry = worker.handle().new_telemetry(endpoints);
			Ok((worker, telemetry))
		})
		.transpose()?;

	let heap_pages = config
		.default_heap_pages
		.map_or(DEFAULT_HEAP_ALLOC_STRATEGY, |h| HeapAllocStrategy::Static { extra_pages: h as _ });

	let wasm = WasmExecutor::builder()
		.with_execution_method(config.wasm_method)
		.with_onchain_heap_alloc_strategy(heap_pages)
		.with_offchain_heap_alloc_strategy(heap_pages)
		.with_max_runtime_instances(config.max_runtime_instances)
		.with_runtime_cache_size(config.runtime_cache_size)
		.build();

	let executor = ParachainExecutor::new_with_wasm_executor(wasm);

	let (client, backend, keystore_container, task_manager) =
		sc_service::new_full_parts::<Block, RuntimeApi, _>(
			config,
			telemetry.as_ref().map(|(_, telemetry)| telemetry.handle()),
			executor,
		)?;
	let client = Arc::new(client);

	let telemetry_worker_handle = telemetry.as_ref().map(|(worker, _)| worker.handle());

	let telemetry = telemetry.map(|(worker, telemetry)| {
		task_manager.spawn_handle().spawn("telemetry", None, worker.run());
		telemetry
	});

	let transaction_pool = sc_transaction_pool::BasicPool::new_full(
		config.transaction_pool.clone(),
		config.role.is_authority().into(),
		config.prometheus_registry(),
		task_manager.spawn_essential_handle(),
		client.clone(),
	);

	let overrides = crate::rpc::overrides_handle(client.clone());
	let frontier_backend = match eth_config.frontier_backend_type {
		BackendType::KeyValue => FrontierBackend::KeyValue(fc_db::kv::Backend::open(
			Arc::clone(&client),
			&config.database,
			&db_config_dir(config),
		)?),
		BackendType::Sql => {
			let db_path = db_config_dir(config).join("sql");
			std::fs::create_dir_all(&db_path).expect("failed creating sql db directory");
			let backend = futures::executor::block_on(fc_db::sql::Backend::new(
				fc_db::sql::BackendConfig::Sqlite(fc_db::sql::SqliteBackendConfig {
					path: Path::new("sqlite:///")
						.join(db_path)
						.join("frontier.db3")
						.to_str()
						.unwrap(),
					create_if_missing: true,
					thread_count: eth_config.frontier_sql_backend_thread_count,
					cache_size: eth_config.frontier_sql_backend_cache_size,
				}),
				eth_config.frontier_sql_backend_pool_size,
				std::num::NonZeroU32::new(eth_config.frontier_sql_backend_num_ops_timeout),
				overrides.clone(),
			))
			.unwrap_or_else(|err| panic!("failed creating sql backend: {:?}", err));
			FrontierBackend::Sql(backend)
		},
	};

	let frontier_block_import = FrontierBlockImport::new(client.clone(), client.clone());

	let parachain_block_import = ParachainBlockImport::new(frontier_block_import, backend.clone());

	let import_queue = build_import_queue(
		client.clone(),
		parachain_block_import.clone(),
		config,
		eth_config,
		telemetry.as_ref().map(|telemetry| telemetry.handle()),
		&task_manager,
	)?;

	Ok(PartialComponents {
		backend,
		client,
		import_queue,
		keystore_container,
		task_manager,
		transaction_pool,
		select_chain: (),
		other: (
			parachain_block_import,
			telemetry,
			telemetry_worker_handle,
			frontier_backend,
			overrides,
		),
	})
}

/// Start a node with the given parachain `Configuration` and relay chain `Configuration`.
///
/// This is the actual implementation that is abstract over the executor and the runtime api.
#[sc_tracing::logging::prefix_logs_with("Parachain")]
async fn start_node_impl(
	parachain_config: Configuration,
	polkadot_config: Configuration,
	eth_config: EthConfiguration,
	collator_options: CollatorOptions,
	para_id: ParaId,
	hwbench: Option<sc_sysinfo::HwBench>,
) -> sc_service::error::Result<(TaskManager, Arc<ParachainClient>)> {
	let mut parachain_config = prepare_node_config(parachain_config);

	let PartialComponents {
		client,
		backend,
		mut task_manager,
		import_queue,
		keystore_container,
		transaction_pool,
		other: (block_import, mut telemetry, telemetry_worker_handle, frontier_backend, overrides),
		..
	} = new_partial(&parachain_config, &eth_config)?;

	let FrontierPartialComponents { filter_pool, fee_history_cache, fee_history_cache_limit } =
		new_frontier_partial(&eth_config)?;

	let (relay_chain_interface, collator_key) = build_relay_chain_interface(
		polkadot_config,
		&parachain_config,
		telemetry_worker_handle,
		&mut task_manager,
		collator_options.clone(),
		hwbench.clone(),
	)
	.await
	.map_err(|e| sc_service::Error::Application(Box::new(e) as Box<_>))?;

	let force_authoring = parachain_config.force_authoring;
	let validator = parachain_config.role.is_authority();
	let prometheus_registry = parachain_config.prometheus_registry().cloned();
	let import_queue_service = import_queue.service();
	let net_config = sc_network::config::FullNetworkConfiguration::new(&parachain_config.network);

	let (network, system_rpc_tx, tx_handler_controller, start_network, sync_service) =
		build_network(BuildNetworkParams {
			parachain_config: &parachain_config,
			client: client.clone(),
			transaction_pool: transaction_pool.clone(),
			para_id,
			net_config,
			spawn_handle: task_manager.spawn_handle(),
			relay_chain_interface: relay_chain_interface.clone(),
			import_queue,
		})
		.await?;

	if parachain_config.offchain_worker.enabled {
		task_manager.spawn_handle().spawn(
			"offchain-workers-runner",
			"offchain-worker",
			sc_offchain::OffchainWorkers::new(sc_offchain::OffchainWorkerOptions {
				runtime_api_provider: client.clone(),
				is_validator: parachain_config.role.is_authority(),
				keystore: Some(keystore_container.keystore()),
				offchain_db: backend.offchain_storage(),
				transaction_pool: Some(OffchainTransactionPoolFactory::new(
					transaction_pool.clone(),
				)),
				network_provider: network.clone(),
				enable_http_requests: true,
				custom_extensions: |_| vec![],
			})
			.run(client.clone(), task_manager.spawn_handle())
			.boxed(),
		);
	}

	// Sinks for pubsub notifications.
	// Everytime a new subscription is created, a new mpsc channel is added to the sink pool.
	// The MappingSyncWorker sends through the channel on block import and the subscription emits a notification to the subscriber on receiving a message through this channel.
	// This way we avoid race conditions when using native substrate block import notification stream.
	let pubsub_notification_sinks: fc_mapping_sync::EthereumBlockNotificationSinks<
		fc_mapping_sync::EthereumBlockNotification<Block>,
	> = Default::default();
	let pubsub_notification_sinks = Arc::new(pubsub_notification_sinks);

	// for ethereum-compatibility rpc.
	parachain_config.rpc_id_provider = Some(Box::new(fc_rpc::EthereumSubIdProvider));

	let eth_rpc_params = crate::rpc::EthDeps {
		client: client.clone(),
		pool: transaction_pool.clone(),
		graph: transaction_pool.pool().clone(),
		converter: Some(TransactionConverter),
		is_authority: parachain_config.role.is_authority(),
		enable_dev_signer: eth_config.enable_dev_signer,
		network: network.clone(),
		sync: sync_service.clone(),
		frontier_backend: match frontier_backend.clone() {
			fc_db::Backend::KeyValue(b) => Arc::new(b),
			fc_db::Backend::Sql(b) => Arc::new(b),
		},
		overrides: overrides.clone(),
		block_data_cache: Arc::new(fc_rpc::EthBlockDataCacheTask::new(
			task_manager.spawn_handle(),
			overrides.clone(),
			eth_config.eth_log_block_cache,
			eth_config.eth_statuses_cache,
			prometheus_registry.clone(),
		)),
		filter_pool: filter_pool.clone(),
		max_past_logs: eth_config.max_past_logs,
		fee_history_cache: fee_history_cache.clone(),
		fee_history_cache_limit,
		execute_gas_limit_multiplier: eth_config.execute_gas_limit_multiplier,
		forced_parent_hashes: None,
	};

	let rpc_builder = {
		let client = client.clone();
		let transaction_pool = transaction_pool.clone();
		let pubsub_notification_sinks = pubsub_notification_sinks.clone();

		Box::new(move |deny_unsafe, subscription_task_executor| {
			let deps = crate::rpc::FullDeps {
				client: client.clone(),
				pool: transaction_pool.clone(),
				deny_unsafe,
				eth: eth_rpc_params.clone(),
			};

			crate::rpc::create_full(
				deps,
				subscription_task_executor,
				pubsub_notification_sinks.clone(),
			)
			.map_err(Into::into)
		})
	};

	let _rpc_handlers = sc_service::spawn_tasks(sc_service::SpawnTasksParams {
		config: parachain_config,
		client: client.clone(),
		backend: backend.clone(),
		task_manager: &mut task_manager,
		keystore: keystore_container.keystore(),
		transaction_pool: transaction_pool.clone(),
		rpc_builder,
		network: network.clone(),
		system_rpc_tx,
		tx_handler_controller,
		sync_service: sync_service.clone(),
		telemetry: telemetry.as_mut(),
	})?;

	spawn_frontier_tasks(
		&task_manager,
		client.clone(),
		backend,
		frontier_backend,
		filter_pool,
		overrides,
		fee_history_cache,
		fee_history_cache_limit,
		sync_service.clone(),
		pubsub_notification_sinks,
	)
	.await;

	if let Some(hwbench) = hwbench {
		sc_sysinfo::print_hwbench(&hwbench);
		// Here you can check whether the hardware meets your chains' requirements. Putting a link
		// in there and swapping out the requirements for your own are probably a good idea. The
		// requirements for a para-chain are dictated by its relay-chain.
		if !SUBSTRATE_REFERENCE_HARDWARE.check_hardware(&hwbench) && validator {
			log::warn!(
				"⚠️  The hardware does not meet the minimal requirements for role 'Authority'."
			);
		}

		if let Some(ref mut telemetry) = telemetry {
			let telemetry_handle = telemetry.handle();
			task_manager.spawn_handle().spawn(
				"telemetry_hwbench",
				None,
				sc_sysinfo::initialize_hwbench_telemetry(telemetry_handle, hwbench),
			);
		}
	}

	let announce_block = {
		let sync_service = sync_service.clone();
		Arc::new(move |hash, data| sync_service.announce_block(hash, data))
	};

	let relay_chain_slot_duration = Duration::from_secs(6);

	let overseer_handle = relay_chain_interface
		.overseer_handle()
		.map_err(|e| sc_service::Error::Application(Box::new(e)))?;

	if validator {
		let parachain_consensus = build_consensus(
			client.clone(),
			block_import,
			prometheus_registry.as_ref(),
			telemetry.as_ref().map(|t| t.handle()),
			&task_manager,
			relay_chain_interface.clone(),
			transaction_pool,
			sync_service.clone(),
			keystore_container.keystore(),
			force_authoring,
			para_id,
		)?;

		let spawner = task_manager.spawn_handle();
		let params = StartCollatorParams {
			para_id,
			block_status: client.clone(),
			announce_block,
			client: client.clone(),
			task_manager: &mut task_manager,
			relay_chain_interface,
			spawner,
			parachain_consensus,
			import_queue: import_queue_service,
			collator_key: collator_key.expect("Command line arguments do not allow this. qed"),
			relay_chain_slot_duration,
			recovery_handle: Box::new(overseer_handle),
			sync_service,
		};

		start_collator(params).await?;
	} else {
		let params = StartFullNodeParams {
			client: client.clone(),
			announce_block,
			task_manager: &mut task_manager,
			para_id,
			relay_chain_interface,
			relay_chain_slot_duration,
			import_queue: import_queue_service,
			recovery_handle: Box::new(overseer_handle),
			sync_service,
		};

		start_full_node(params)?;
	}

	start_network.start_network();

	Ok((task_manager, client))
}

/// Build the import queue for the parachain runtime.
fn build_import_queue(
	client: Arc<ParachainClient>,
	block_import: ParachainBlockImport,
	config: &Configuration,
	eth_config: &EthConfiguration,
	telemetry: Option<TelemetryHandle>,
	task_manager: &TaskManager,
) -> Result<sc_consensus::DefaultImportQueue<Block, ParachainClient>, sc_service::Error> {
	let slot_duration = cumulus_client_consensus_aura::slot_duration(&*client)?;
	let target_gas_price = eth_config.target_gas_price;

	cumulus_client_consensus_aura::import_queue::<
		sp_consensus_aura::sr25519::AuthorityPair,
		_,
		_,
		_,
		_,
		_,
	>(cumulus_client_consensus_aura::ImportQueueParams {
		block_import,
		client,
		create_inherent_data_providers: move |_, _| async move {
			let timestamp = sp_timestamp::InherentDataProvider::from_system_time();
			let slot =
				sp_consensus_aura::inherents::InherentDataProvider::from_timestamp_and_slot_duration(
					*timestamp,
					slot_duration,
				);
			let dynamic_fee = fp_dynamic_fee::InherentDataProvider(U256::from(target_gas_price));
			Ok((slot, timestamp, dynamic_fee))
		},
		registry: config.prometheus_registry(),
		spawner: &task_manager.spawn_essential_handle(),
		telemetry,
	})
	.map_err(Into::into)
}

fn build_consensus(
	client: Arc<ParachainClient>,
	block_import: ParachainBlockImport,
	prometheus_registry: Option<&Registry>,
	telemetry: Option<TelemetryHandle>,
	task_manager: &TaskManager,
	relay_chain_interface: Arc<dyn RelayChainInterface>,
	transaction_pool: Arc<sc_transaction_pool::FullPool<Block, ParachainClient>>,
	sync_oracle: Arc<SyncingService<Block>>,
	keystore: KeystorePtr,
	force_authoring: bool,
	para_id: ParaId,
) -> Result<Box<dyn ParachainConsensus<Block>>, sc_service::Error> {
	let slot_duration = cumulus_client_consensus_aura::slot_duration(&*client)?;

	let proposer_factory = sc_basic_authorship::ProposerFactory::with_proof_recording(
		task_manager.spawn_handle(),
		client.clone(),
		transaction_pool,
		prometheus_registry,
		telemetry.clone(),
	);

	let params = BuildAuraConsensusParams {
		proposer_factory,
		create_inherent_data_providers: move |_, (relay_parent, validation_data)| {
			let relay_chain_interface = relay_chain_interface.clone();
			async move {
				let parachain_inherent =
					cumulus_primitives_parachain_inherent::ParachainInherentData::create_at(
						relay_parent,
						&relay_chain_interface,
						&validation_data,
						para_id,
					)
					.await;
				let timestamp = sp_timestamp::InherentDataProvider::from_system_time();

				let slot =
						sp_consensus_aura::inherents::InherentDataProvider::from_timestamp_and_slot_duration(
							*timestamp,
							slot_duration,
						);

				let parachain_inherent = parachain_inherent.ok_or_else(|| {
					Box::<dyn std::error::Error + Send + Sync>::from(
						"Failed to create parachain inherent",
					)
				})?;
				Ok((slot, timestamp, parachain_inherent))
			}
		},
		block_import,
		para_client: client,
		backoff_authoring_blocks: Option::<()>::None,
		sync_oracle,
		keystore,
		force_authoring,
		slot_duration,
		// We got around 500ms for proposing
		block_proposal_slot_portion: SlotProportion::new(1f32 / 24f32),
		// And a maximum of 750ms if slots are skipped
		max_block_proposal_slot_portion: Some(SlotProportion::new(1f32 / 16f32)),
		telemetry,
	};

	Ok(AuraConsensus::build::<sp_consensus_aura::sr25519::AuthorityPair, _, _, _, _, _, _>(params))
}

/// Start a parachain node.
pub async fn start_parachain_node(
	parachain_config: Configuration,
	polkadot_config: Configuration,
	eth_config: EthConfiguration,
	collator_options: CollatorOptions,
	para_id: ParaId,
	hwbench: Option<sc_sysinfo::HwBench>,
) -> sc_service::error::Result<(TaskManager, Arc<ParachainClient>)> {
	start_node_impl(
		parachain_config,
		polkadot_config,
		eth_config,
		collator_options,
		para_id,
		hwbench,
	)
	.await
}
