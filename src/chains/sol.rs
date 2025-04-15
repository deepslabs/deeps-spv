use crate::{config, SOL_RUNTIME};
use solana_client::rpc_client::{GetConfirmedSignaturesForAddress2Config, RpcClient};
use solana_client::rpc_config::{
    RpcAccountInfoConfig, RpcProgramAccountsConfig, RpcTransactionConfig,
};
use solana_client::rpc_response::RpcConfirmedTransactionStatusWithSignature;
use solana_sdk::account::Account;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_transaction_status_client_types::{
    EncodedConfirmedTransactionWithStatusMeta, UiTransactionEncoding,
};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{error, info};

enum TaskEvent {
    GetTransaction(
        Signature,
        Option<RpcTransactionConfig>,
        tokio::sync::mpsc::Sender<ResponseEvent>,
    ),
    GetSlot(
        Option<CommitmentConfig>,
        tokio::sync::mpsc::Sender<ResponseEvent>,
    ),
    GetAccountInfo(
        Pubkey,
        Option<RpcAccountInfoConfig>,
        tokio::sync::mpsc::Sender<ResponseEvent>,
    ),
    GetSignaturesForAddress(
        Pubkey,
        Option<GetConfirmedSignaturesForAddress2Config>,
        tokio::sync::mpsc::Sender<ResponseEvent>,
    ),
    GetProgramAccounts(
        Pubkey,
        Option<RpcProgramAccountsConfig>,
        tokio::sync::mpsc::Sender<ResponseEvent>,
    ),
}

#[derive(Clone)]
enum InnerTaskEvent {
    /// GetTransaction(signature, get_tx_config, sender(res, from_rpc)
    GetTransaction(
        Signature,
        Option<RpcTransactionConfig>,
        tokio::sync::mpsc::Sender<(
            Result<EncodedConfirmedTransactionWithStatusMeta, SolClientError>,
            String,
        )>,
    ),

    /// GetSlot(CommitmentConfig)
    GetSlot(
        Option<CommitmentConfig>,
        tokio::sync::mpsc::Sender<(Result<u64, SolClientError>, String)>,
    ),

    GetAccountInfo(
        Pubkey,
        Option<RpcAccountInfoConfig>,
        tokio::sync::mpsc::Sender<(Result<Option<Account>, SolClientError>, String)>,
    ),

    GetSignaturesForAddress(
        Pubkey,
        Option<(
            Option<Signature>,
            Option<Signature>,
            Option<usize>,
            Option<CommitmentConfig>,
        )>,
        tokio::sync::mpsc::Sender<(
            Result<Vec<RpcConfirmedTransactionStatusWithSignature>, SolClientError>,
            String,
        )>,
    ),

    GetProgramAccounts(
        Pubkey,
        Option<RpcProgramAccountsConfig>,
        tokio::sync::mpsc::Sender<(Result<Vec<(Pubkey, Account)>, SolClientError>, String)>,
    ),
}

pub(crate) enum ResponseEvent {
    /// GetTransaction(res)
    GetTransaction(Result<EncodedConfirmedTransactionWithStatusMeta, SolClientError>),
    GetSlot(Result<u64, SolClientError>),
    GetAccountInfo(Result<Option<Account>, SolClientError>),
    GetSignaturesForAddress(
        Result<Vec<RpcConfirmedTransactionStatusWithSignature>, SolClientError>,
    ),
    GetProgramAccounts(Result<Vec<(Pubkey, Account)>, SolClientError>),
}

#[derive(Debug)]
pub(crate) enum SolClientError {
    Custom(String),
    Internal(String),
    TimeOut(String),
}

impl Display for SolClientError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            SolClientError::Custom(err) => {
                write!(f, "{}", err)
            }
            SolClientError::Internal(_err) => {
                write!(f, "InternalError")
            }
            SolClientError::TimeOut(_err) => {
                write!(f, "TimeOut")
            }
        }
    }
}

pub(crate) struct SolClient {
    sol_extra_rpc: Vec<Arc<RpcClient>>,
    sol_judge_rpc: RpcClient,
    task_sender: tokio::sync::mpsc::Sender<TaskEvent>,
    task_receiver: Arc<Mutex<tokio::sync::mpsc::Receiver<TaskEvent>>>,
}

impl SolClient {
    pub(crate) fn initialize_sol_client(
        config: &config::Config,
    ) -> Result<Option<Arc<SolClient>>, String> {
        let sol_extra_rpcs = config
            .configcli
            .sol_extra_rpcs
            .split(",")
            .map(|u| u.to_string())
            .collect::<Vec<_>>();
        let sol_judge_rpc = &config.configcli.sol_judge_rpc;

        if config.service.sol {
            let unique_urls: HashSet<_> = sol_extra_rpcs.iter().collect();
            if env::var("ENABLE_SINGLE_SOL_RPC").is_err() {
                if unique_urls.len() < 3 {
                    return Err("sol urls num mut >= 3".to_string());
                }

                if unique_urls.len() % 2 == 0 {
                    return Err("urls num mut be odd number".to_string());
                }
            }

            info!(target: "spv","sol judge rpc: {}", sol_judge_rpc);
            unique_urls.iter().for_each(|url| {
                info!(target: "spv","sol extra rpc: {}", url);
            });

            let judge_client = RpcClient::new(sol_judge_rpc);

            let mut extra_clients = unique_urls
                .into_iter()
                .map(RpcClient::new)
                .collect::<Vec<_>>();

            extra_clients.push(judge_client);
            if !Self::is_all_client_available(&extra_clients) {
                return Err("sol clients are not all available".to_string());
            }

            let (task_sender, task_receiver) = tokio::sync::mpsc::channel::<TaskEvent>(1024);

            let sol_client = Arc::new(Self {
                sol_judge_rpc: extra_clients.pop().unwrap(),
                sol_extra_rpc: extra_clients.into_iter().map(Arc::new).collect(),
                task_sender,
                task_receiver: Arc::new(Mutex::new(task_receiver)),
            });

            SOL_RUNTIME.block_on(async {
                let dispatcher_sender = sol_client.clone().start_dispatcher().await;
                sol_client
                    .clone()
                    .start_client_workers(dispatcher_sender)
                    .await;
            });

            Ok(Some(sol_client))
        } else {
            Ok(None)
        }
    }

    async fn start_dispatcher(&self) -> tokio::sync::broadcast::Sender<InnerTaskEvent> {
        let task_receiver = self.task_receiver.clone();
        let (dispatcher_sender, _) = tokio::sync::broadcast::channel(1024);
        let dispatcher_sender_cloned = dispatcher_sender.clone();
        let rpc_num = self.sol_extra_rpc.len();
        SOL_RUNTIME.spawn(async move {
            loop {
                if let Some(task) = task_receiver.lock().await.recv().await {
                    match task {
                        TaskEvent::GetTransaction(signature, rpc_tx_config, response_sender) => {
                            Self::deal_get_transaction(
                                rpc_num,
                                signature,
                                rpc_tx_config,
                                dispatcher_sender_cloned.clone(),
                                response_sender,
                            )
                            .await;
                        }
                        TaskEvent::GetSlot(commitment_config, response_sender) => {
                            Self::deal_get_slot(
                                rpc_num,
                                commitment_config,
                                dispatcher_sender_cloned.clone(),
                                response_sender,
                            )
                            .await;
                        }
                        TaskEvent::GetAccountInfo(pubkey, config, response_sender) => {
                            Self::deal_get_account_info(
                                rpc_num,
                                pubkey,
                                config,
                                dispatcher_sender_cloned.clone(),
                                response_sender,
                            )
                            .await;
                        }
                        TaskEvent::GetSignaturesForAddress(pubkey, config, response_sender) => {
                            Self::deal_get_signatures_for_address(
                                rpc_num,
                                pubkey,
                                config,
                                dispatcher_sender_cloned.clone(),
                                response_sender,
                            )
                            .await;
                        }
                        TaskEvent::GetProgramAccounts(pubkey, config, response_sender) => {
                            Self::deal_get_program_accounts(
                                rpc_num,
                                pubkey,
                                config,
                                dispatcher_sender_cloned.clone(),
                                response_sender,
                            )
                            .await;
                        }
                    }
                }
            }
        });
        dispatcher_sender
    }

    async fn start_client_workers(
        &self,
        dispatcher_sender: tokio::sync::broadcast::Sender<InnerTaskEvent>,
    ) {
        let clients = self.sol_extra_rpc.clone();
        clients.into_iter().for_each(|client| {
            let mut dispatcher_receiver = dispatcher_sender.subscribe();
            let url = client.url();
            SOL_RUNTIME.spawn(async move {
                loop {
                    if let Ok(task) = dispatcher_receiver.recv().await {
                        match task {
                            InnerTaskEvent::GetTransaction(signature, tx_config, inner_sender) => {
                                Self::client_worker_do_get_transaction(
                                    client.clone(),
                                    tx_config,
                                    &signature,
                                    url.clone(),
                                    inner_sender,
                                )
                                .await
                            }
                            InnerTaskEvent::GetSlot(commitment_config, inner_sender) => {
                                Self::client_worker_do_get_slot(
                                    client.clone(),
                                    commitment_config,
                                    url.clone(),
                                    inner_sender,
                                )
                                .await
                            }
                            InnerTaskEvent::GetAccountInfo(pubkey, config, inner_sender) => {
                                Self::client_worker_do_get_account(
                                    client.clone(),
                                    config,
                                    url.clone(),
                                    inner_sender,
                                    pubkey,
                                )
                                .await
                            }
                            InnerTaskEvent::GetSignaturesForAddress(
                                pubkey,
                                config,
                                inner_sender,
                            ) => {
                                Self::client_worker_do_get_signatures_for_address(
                                    client.clone(),
                                    config.map(|config| GetConfirmedSignaturesForAddress2Config {
                                        before: config.0,
                                        until: config.1,
                                        limit: config.2,
                                        commitment: config.3,
                                    }),
                                    url.clone(),
                                    inner_sender,
                                    pubkey,
                                )
                                .await
                            }
                            InnerTaskEvent::GetProgramAccounts(pubkey, config, inner_sender) => {
                                Self::client_worker_do_get_program_accounts(
                                    client.clone(),
                                    config,
                                    url.clone(),
                                    inner_sender,
                                    pubkey,
                                )
                                .await
                            }
                        }
                    }
                }
            });
        });
    }

    async fn client_worker_do_get_program_accounts(
        client: Arc<RpcClient>,
        config: Option<RpcProgramAccountsConfig>,
        url: String,
        inner_sender: tokio::sync::mpsc::Sender<(
            Result<Vec<(Pubkey, Account)>, SolClientError>,
            String,
        )>,
        pubkey: Pubkey,
    ) {
        let mut task_success = false;
        for _ in 0..3 {
            let get_res = match &config {
                None => client.get_program_accounts(&pubkey),
                Some(config) => client.get_program_accounts_with_config(&pubkey, config.clone()),
            };
            match get_res {
                Ok(accounts) => {
                    if let Err(e) = inner_sender.send((Ok(accounts), url.clone())).await {
                        error!("{e:?}")
                    }
                    task_success = true;
                    break;
                }
                Err(err) => {
                    error!("failed to get program_accounts {pubkey} from {url}, error: {err}");
                    continue;
                }
            }
        }
        if !task_success {
            if let Err(e) = inner_sender
                .send((
                    Err(SolClientError::Internal(format!(
                        "failed to get program_accounts {pubkey} from {url}, retry limited"
                    ))),
                    url.clone(),
                ))
                .await
            {
                error!("{e:?}");
            };
        }
    }

    async fn client_worker_do_get_signatures_for_address(
        client: Arc<RpcClient>,
        config: Option<GetConfirmedSignaturesForAddress2Config>,
        url: String,
        inner_sender: tokio::sync::mpsc::Sender<(
            Result<Vec<RpcConfirmedTransactionStatusWithSignature>, SolClientError>,
            String,
        )>,
        pubkey: Pubkey,
    ) {
        let mut task_success = false;
        for _ in 0..3 {
            let get_res = match &config {
                None => client.get_signatures_for_address(&pubkey),
                Some(config) => client.get_signatures_for_address_with_config(
                    &pubkey,
                    GetConfirmedSignaturesForAddress2Config {
                        before: config.before,
                        until: config.until,
                        limit: config.limit,
                        commitment: config.commitment,
                    },
                ),
            };
            match get_res {
                Ok(signatures) => {
                    if let Err(e) = inner_sender.send((Ok(signatures), url.clone())).await {
                        error!("{e:?}")
                    }
                    task_success = true;
                    break;
                }
                Err(err) => {
                    error!("failed to get signatures from {url}, error: {err}");
                    continue;
                }
            }
        }
        if !task_success {
            if let Err(e) = inner_sender
                .send((
                    Err(SolClientError::Internal(format!(
                        "failed to get signatures from {url}, retry limited"
                    ))),
                    url.clone(),
                ))
                .await
            {
                error!("{e:?}");
            };
        }
    }

    async fn client_worker_do_get_account(
        client: Arc<RpcClient>,
        config: Option<RpcAccountInfoConfig>,
        url: String,
        inner_sender: tokio::sync::mpsc::Sender<(Result<Option<Account>, SolClientError>, String)>,
        pubkey: Pubkey,
    ) {
        let mut task_success = false;
        for _ in 0..3 {
            let get_res = match &config {
                None => match client.get_account(&pubkey) {
                    Ok(account) => Ok(Some(account)),
                    Err(err) => {
                        if let Some(err) = err.get_transaction_error() {
                            let err_s = err.to_string();
                            if err_s.contains("AccountNotFound") {
                                Ok(None)
                            } else {
                                Err(err_s)
                            }
                        } else {
                            Err(format!("{err:?}"))
                        }
                    }
                },
                Some(config) => match client.get_account_with_config(&pubkey, config.clone()) {
                    Ok(res) => Ok(res.value),
                    Err(err) => Err(format!("{err:?}")),
                },
            };
            match get_res {
                Ok(account) => {
                    if let Err(e) = inner_sender.send((Ok(account), url.clone())).await {
                        error!("{e:?}")
                    }
                    task_success = true;
                    break;
                }
                Err(err) => {
                    error!("failed to get account from {url}, error: {err}");
                    continue;
                }
            }
        }
        if !task_success {
            if let Err(e) = inner_sender
                .send((
                    Err(SolClientError::Internal(format!(
                        "failed to get account from {url}, retry limited"
                    ))),
                    url.clone(),
                ))
                .await
            {
                error!("{e:?}");
            };
        }
    }

    async fn client_worker_do_get_slot(
        client: Arc<RpcClient>,
        commitment_config: Option<CommitmentConfig>,
        url: String,
        inner_sender: tokio::sync::mpsc::Sender<(Result<u64, SolClientError>, String)>,
    ) {
        let mut task_success = false;
        for _ in 0..3 {
            let get_res = match commitment_config {
                None => client.get_slot(),
                Some(config) => client.get_slot_with_commitment(config),
            };
            match get_res {
                Ok(slot) => {
                    if let Err(e) = inner_sender.send((Ok(slot), url.clone())).await {
                        error!("{e:?}")
                    }
                    task_success = true;
                    break;
                }
                Err(err) => {
                    error!("failed to get slot from {url}, error: {err:?}");
                    continue;
                }
            }
        }
        if !task_success {
            if let Err(e) = inner_sender
                .send((
                    Err(SolClientError::Internal(format!(
                        "failed to get slot from {url}, retry limited"
                    ))),
                    url.clone(),
                ))
                .await
            {
                error!("{e:?}");
            };
        }
    }

    async fn client_worker_do_get_transaction(
        client: Arc<RpcClient>,
        tx_config: Option<RpcTransactionConfig>,
        signature: &Signature,
        url: String,
        inner_sender: tokio::sync::mpsc::Sender<(
            Result<EncodedConfirmedTransactionWithStatusMeta, SolClientError>,
            String,
        )>,
    ) {
        let mut task_success = false;
        for _ in 0..3 {
            let get_res = match tx_config {
                None => client.get_transaction(signature, UiTransactionEncoding::Json),
                Some(config) => client.get_transaction_with_config(signature, config),
            };
            match get_res {
                Ok(tx) => {
                    if let Err(e) = inner_sender.send((Ok(tx), url.clone())).await {
                        error!("{e:?}")
                    }
                    task_success = true;
                    break;
                }
                Err(err) => {
                    error!("failed to get transaction {signature}, from {url}, error: {err:?}");
                    continue;
                }
            }
        }
        if !task_success {
            if let Err(e) = inner_sender
                .send((
                    Err(SolClientError::Internal(format!(
                        "failed to get transaction from {url}, retry limited"
                    ))),
                    url.clone(),
                ))
                .await
            {
                error!("{e:?}");
            };
        }
    }

    async fn deal_get_program_accounts(
        rpc_num: usize,
        pubkey: Pubkey,
        config: Option<RpcProgramAccountsConfig>,
        dispatcher_sender: tokio::sync::broadcast::Sender<InnerTaskEvent>,
        response_sender: tokio::sync::mpsc::Sender<ResponseEvent>,
    ) {
        let (inner_sender, mut inner_receiver) = tokio::sync::mpsc::channel(rpc_num);
        match dispatcher_sender.send(InnerTaskEvent::GetProgramAccounts(
            pubkey,
            config,
            inner_sender,
        )) {
            Ok(_) => {
                let start_time = Instant::now();
                let mut res = HashMap::new();
                loop {
                    if let Ok(Some((result, from))) =
                        tokio::time::timeout(Duration::from_secs(1), inner_receiver.recv()).await
                    {
                        match result {
                            Ok(accounts) => {
                                info!(target: "spv", "GetProgramAccounts from {}", from);
                                res.insert(from, accounts);
                            }
                            Err(err) => {
                                if let Err(e) = response_sender
                                    .send(ResponseEvent::GetProgramAccounts(Err(
                                        SolClientError::Internal(format!(
                                            "GetAccountInfo received error from {}: {}",
                                            from, err
                                        )),
                                    )))
                                    .await
                                {
                                    error!("{e:?}");
                                };
                                break;
                            }
                        };

                        if res.len() == rpc_num {
                            // deal results
                            match Self::deal_get_program_accounts_res(res) {
                                Ok(account) => {
                                    if let Err(e) = response_sender
                                        .send(ResponseEvent::GetProgramAccounts(Ok(account)))
                                        .await
                                    {
                                        error!("{e:?}");
                                    }
                                }
                                Err(err) => {
                                    if let Err(e) = response_sender
                                        .send(ResponseEvent::GetProgramAccounts(Err(err)))
                                        .await
                                    {
                                        error!("{e:?}");
                                    }
                                }
                            }
                            break;
                        }

                        if Instant::now().duration_since(start_time) >= Duration::from_secs(60) {
                            if let Err(e) = response_sender.send(
                                ResponseEvent::GetProgramAccounts(
                                    Err(SolClientError::TimeOut(
                                        format!(
                                            "GetProgramAccounts cancelled, took more than 60s, received from: {:?}",
                                            res.iter().map(|r| r.1.clone()).collect::<Vec<_>>()
                                        )
                                    )))
                            ).await {
                                error!("{e:?}")
                            }
                            break;
                        }
                    }
                }
            }
            Err(err) => {
                if let Err(e) = response_sender
                    .send(ResponseEvent::GetProgramAccounts(Err(
                        SolClientError::Internal(err.to_string()),
                    )))
                    .await
                {
                    error!(target: "spv", "{e:?}")
                }
            }
        }
    }
    async fn deal_get_signatures_for_address(
        rpc_num: usize,
        pubkey: Pubkey,
        config: Option<GetConfirmedSignaturesForAddress2Config>,
        dispatcher_sender: tokio::sync::broadcast::Sender<InnerTaskEvent>,
        response_sender: tokio::sync::mpsc::Sender<ResponseEvent>,
    ) {
        let (inner_sender, mut inner_receiver) = tokio::sync::mpsc::channel(rpc_num);
        match dispatcher_sender.send(InnerTaskEvent::GetSignaturesForAddress(
            pubkey,
            config.map(|c| (c.before, c.until, c.limit, c.commitment)),
            inner_sender,
        )) {
            Ok(_) => {
                let start_time = Instant::now();
                let mut res = HashMap::new();
                loop {
                    if let Ok(Some((result, from))) =
                        tokio::time::timeout(Duration::from_secs(1), inner_receiver.recv()).await
                    {
                        match result {
                            Ok(signatures) => {
                                info!(target: "spv", "GetSignaturesForAddress from {}", from);
                                res.insert(from, signatures);
                            }
                            Err(err) => {
                                if let Err(e) = response_sender
                                    .send(ResponseEvent::GetSignaturesForAddress(Err(
                                        SolClientError::Internal(format!(
                                            "GetSignaturesForAddress received error from {}: {}",
                                            from, err
                                        )),
                                    )))
                                    .await
                                {
                                    error!("{e:?}");
                                };
                                break;
                            }
                        };

                        if res.len() == rpc_num {
                            // deal results
                            match Self::deal_get_signatures_for_address_res(res) {
                                Ok(signatures) => {
                                    if let Err(e) = response_sender
                                        .send(ResponseEvent::GetSignaturesForAddress(Ok(
                                            signatures,
                                        )))
                                        .await
                                    {
                                        error!("{e:?}");
                                    }
                                }
                                Err(err) => {
                                    if let Err(e) = response_sender
                                        .send(ResponseEvent::GetSignaturesForAddress(Err(err)))
                                        .await
                                    {
                                        error!("{e:?}");
                                    }
                                }
                            }
                            break;
                        }

                        if Instant::now().duration_since(start_time) >= Duration::from_secs(60) {
                            if let Err(e) = response_sender.send(
                                ResponseEvent::GetSignaturesForAddress(
                                    Err(SolClientError::TimeOut(
                                        format!(
                                            "GetSignaturesForAddress cancelled, took more than 60s, received from: {:?}",
                                            res.iter().map(|r| r.1.clone()).collect::<Vec<_>>()
                                        )
                                    )))
                            ).await {
                                error!("{e:?}")
                            }
                            break;
                        }
                    }
                }
            }
            Err(err) => {
                if let Err(e) = response_sender
                    .send(ResponseEvent::GetSignaturesForAddress(Err(
                        SolClientError::Internal(err.to_string()),
                    )))
                    .await
                {
                    error!(target: "spv", "{e:?}")
                }
            }
        }
    }

    async fn deal_get_account_info(
        rpc_num: usize,
        pubkey: Pubkey,
        rpc_account_config: Option<RpcAccountInfoConfig>,
        dispatcher_sender: tokio::sync::broadcast::Sender<InnerTaskEvent>,
        response_sender: tokio::sync::mpsc::Sender<ResponseEvent>,
    ) {
        let (inner_sender, mut inner_receiver) = tokio::sync::mpsc::channel(rpc_num);
        match dispatcher_sender.send(InnerTaskEvent::GetAccountInfo(
            pubkey,
            rpc_account_config,
            inner_sender,
        )) {
            Ok(_) => {
                let start_time = Instant::now();
                let mut res = HashSet::new();
                loop {
                    if let Ok(Some((result, from))) =
                        tokio::time::timeout(Duration::from_secs(1), inner_receiver.recv()).await
                    {
                        match result {
                            Ok(account) => {
                                info!(target: "spv", "GetAccountInfo from {}", from);
                                res.insert(((serde_json::to_vec(&account).unwrap()), from));
                            }
                            Err(err) => {
                                if let Err(e) = response_sender
                                    .send(ResponseEvent::GetSlot(Err(SolClientError::Internal(
                                        format!(
                                            "GetAccountInfo received error from {}: {}",
                                            from, err
                                        ),
                                    ))))
                                    .await
                                {
                                    error!("{e:?}");
                                };
                                break;
                            }
                        };

                        if res.len() == rpc_num {
                            // deal results
                            match Self::deal_get_account_res(res) {
                                Ok(account) => {
                                    if let Err(e) = response_sender
                                        .send(ResponseEvent::GetAccountInfo(Ok(account)))
                                        .await
                                    {
                                        error!("{e:?}");
                                    }
                                }
                                Err(err) => {
                                    if let Err(e) = response_sender
                                        .send(ResponseEvent::GetAccountInfo(Err(err)))
                                        .await
                                    {
                                        error!("{e:?}");
                                    }
                                }
                            }
                            break;
                        }

                        if Instant::now().duration_since(start_time) >= Duration::from_secs(60) {
                            if let Err(e) = response_sender.send(
                                ResponseEvent::GetAccountInfo(
                                    Err(SolClientError::TimeOut(
                                        format!(
                                            "GetAccountInfo cancelled, took more than 60s, received from: {:?}",
                                            res.iter().map(|r| r.1.clone()).collect::<Vec<_>>()
                                        )
                                    )))
                            ).await {
                                error!("{e:?}")
                            }
                            break;
                        }
                    }
                }
            }
            Err(err) => {
                if let Err(e) = response_sender
                    .send(ResponseEvent::GetAccountInfo(Err(
                        SolClientError::Internal(err.to_string()),
                    )))
                    .await
                {
                    error!(target: "spv", "{e:?}")
                }
            }
        }
    }

    async fn deal_get_slot(
        rpc_num: usize,
        commitment_config: Option<CommitmentConfig>,
        dispatcher_sender: tokio::sync::broadcast::Sender<InnerTaskEvent>,
        response_sender: tokio::sync::mpsc::Sender<ResponseEvent>,
    ) {
        let (inner_sender, mut inner_receiver) = tokio::sync::mpsc::channel(rpc_num);
        match dispatcher_sender.send(InnerTaskEvent::GetSlot(commitment_config, inner_sender)) {
            Ok(_) => {
                let start_time = Instant::now();
                let mut res = HashSet::new();
                loop {
                    if let Ok(Some((result, from))) =
                        tokio::time::timeout(Duration::from_secs(1), inner_receiver.recv()).await
                    {
                        match result {
                            Ok(slot) => {
                                info!(target: "spv", "GetSlot {} from {}", slot, from);
                                res.insert((slot, from));
                            }
                            Err(err) => {
                                if let Err(e) = response_sender
                                    .send(ResponseEvent::GetSlot(Err(SolClientError::Internal(
                                        format!("GetSlot received error from {}: {}", from, err),
                                    ))))
                                    .await
                                {
                                    error!("{e:?}");
                                };
                                break;
                            }
                        };

                        if res.len() == rpc_num {
                            // deal results
                            match Self::deal_get_slot_res(res) {
                                Ok(slot) => {
                                    if let Err(e) =
                                        response_sender.send(ResponseEvent::GetSlot(Ok(slot))).await
                                    {
                                        error!("{e:?}");
                                    }
                                }
                                Err(err) => {
                                    if let Err(e) =
                                        response_sender.send(ResponseEvent::GetSlot(Err(err))).await
                                    {
                                        error!("{e:?}");
                                    }
                                }
                            }
                            break;
                        }

                        if Instant::now().duration_since(start_time) >= Duration::from_secs(60) {
                            if let Err(e) = response_sender.send(
                                ResponseEvent::GetSlot(
                                    Err(SolClientError::TimeOut(
                                        format!(
                                            "GetSlot cancelled, took more than 60s, received from: {:?}",
                                            res.iter().map(|r| r.1.clone()).collect::<Vec<_>>()
                                        )
                                    )))
                            ).await {
                                error!("{e:?}")
                            }
                            break;
                        }
                    }
                }
            }
            Err(err) => {
                if let Err(e) = response_sender
                    .send(ResponseEvent::GetSlot(Err(SolClientError::Internal(
                        err.to_string(),
                    ))))
                    .await
                {
                    error!(target: "spv", "{e:?}")
                }
            }
        }
    }

    async fn deal_get_transaction(
        rpc_num: usize,
        signature: Signature,
        rpc_tx_config: Option<RpcTransactionConfig>,
        dispatcher_sender: tokio::sync::broadcast::Sender<InnerTaskEvent>,
        response_sender: tokio::sync::mpsc::Sender<ResponseEvent>,
    ) {
        let (inner_sender, mut inner_receiver) = tokio::sync::mpsc::channel(rpc_num);
        match dispatcher_sender.send(InnerTaskEvent::GetTransaction(
            signature,
            rpc_tx_config,
            inner_sender,
        )) {
            Ok(_) => {
                let start_time = Instant::now();
                let mut res = HashSet::new();
                loop {
                    if let Ok(Some((result, from))) =
                        tokio::time::timeout(Duration::from_secs(1), inner_receiver.recv()).await
                    {
                        match result {
                            Ok(tx) => {
                                info!(target: "spv", "GetTransaction {} from {}", signature, from);
                                res.insert((serde_json::to_vec(&tx).unwrap(), from));
                            }
                            Err(err) => {
                                if let Err(e) = response_sender
                                    .send(ResponseEvent::GetTransaction(Err(
                                        SolClientError::Internal(format!(
                                            "GetTransaction received error from {}: {}",
                                            from, err
                                        )),
                                    )))
                                    .await
                                {
                                    error!("{e:?}");
                                };
                                break;
                            }
                        };

                        if res.len() == rpc_num {
                            // deal results
                            match Self::deal_get_transaction_res(res) {
                                Ok(tx) => {
                                    if let Err(e) = response_sender
                                        .send(ResponseEvent::GetTransaction(Ok(tx)))
                                        .await
                                    {
                                        error!("{e:?}");
                                    }
                                }
                                Err(err) => {
                                    if let Err(e) = response_sender
                                        .send(ResponseEvent::GetTransaction(Err(err)))
                                        .await
                                    {
                                        error!("{e:?}");
                                    }
                                }
                            }
                            break;
                        }

                        if Instant::now().duration_since(start_time) >= Duration::from_secs(60) {
                            if let Err(e) = response_sender.send(
                                ResponseEvent::GetTransaction(
                                    Err(SolClientError::TimeOut(
                                        format!(
                                            "GetTransaction cancelled, took more than 60s, received from: {:?}",
                                            res.iter().map(|r| r.1.clone()).collect::<Vec<_>>()
                                        )
                                    )))
                            ).await {
                                error!("{e:?}")
                            }
                            break;
                        }
                    }
                }
            }
            Err(err) => {
                if let Err(e) = response_sender
                    .send(ResponseEvent::GetTransaction(Err(
                        SolClientError::Internal(err.to_string()),
                    )))
                    .await
                {
                    error!(target: "spv", "{e:?}")
                }
            }
        }
    }

    fn deal_get_program_accounts_res(
        accounts: HashMap<String, Vec<(Pubkey, Account)>>,
    ) -> Result<Vec<(Pubkey, Account)>, SolClientError> {
        let mut counts = HashMap::new();
        let members_num = accounts.keys().len();
        accounts
            .into_iter()
            .flat_map(|(_key, accounts)| accounts)
            .collect::<Vec<_>>()
            .into_iter()
            .for_each(|v| {
                counts
                    .entry(serde_json::to_string(&v).unwrap())
                    .and_modify(|counter| *counter += 1)
                    .or_insert(1usize);
            });

        let res = counts
            .into_iter()
            .filter(|(_, v)| *v == members_num)
            .map(|(k, _)| serde_json::from_str(&k).unwrap())
            .collect::<Vec<_>>();

        Ok(res)
    }

    fn deal_get_account_res(
        accounts: HashSet<(Vec<u8>, String)>,
    ) -> Result<Option<Account>, SolClientError> {
        let mut counts = HashMap::new();
        for (account, _from) in accounts {
            counts
                .entry(account)
                .and_modify(|counter| *counter += 1)
                .or_insert(1);
        }

        let max_count = counts.values().cloned().max().unwrap_or(0);
        let max_count_elements = counts.values().filter(|&&count| count == max_count).count();
        if max_count_elements > 1 {
            return Err(SolClientError::Custom(format!(
                "Unexpected max_count_elements {}",
                max_count_elements
            )));
        }

        let account = counts
            .into_iter()
            .max_by_key(|&(_, count)| count)
            .map(|(info, _)| info)
            .ok_or(SolClientError::Custom(
                "Can not find the most suitable account".to_string(),
            ))?;

        Ok(serde_json::from_slice(&account).unwrap())
    }

    fn deal_get_slot_res(slots: HashSet<(u64, String)>) -> Result<u64, SolClientError> {
        let mut counts = HashMap::new();
        for (slot, _from) in slots {
            counts
                .entry(slot)
                .and_modify(|counter| *counter += 1)
                .or_insert(1);
        }

        let max_count = counts.values().cloned().max().unwrap_or(0);
        let max_count_elements = counts.values().filter(|&&count| count == max_count).count();
        if max_count_elements > 1 {
            return Err(SolClientError::Custom(format!(
                "Unexpected max_count_elements {}",
                max_count_elements
            )));
        }

        let slot = counts
            .into_iter()
            .max_by_key(|&(_, count)| count)
            .map(|(info, _)| info)
            .ok_or(SolClientError::Custom(
                "Can not find the most suitable slot".to_string(),
            ))?;

        Ok(slot)
    }

    fn deal_get_transaction_res(
        txs: HashSet<(Vec<u8>, String)>,
    ) -> Result<EncodedConfirmedTransactionWithStatusMeta, SolClientError> {
        let mut counts = HashMap::new();
        for (tx, _from) in txs {
            counts
                .entry(tx)
                .and_modify(|counter| *counter += 1)
                .or_insert(1);
        }

        let max_count = counts.values().cloned().max().unwrap_or(0);
        let max_count_elements = counts.values().filter(|&&count| count == max_count).count();
        if max_count_elements > 1 {
            return Err(SolClientError::Custom(format!(
                "Unexpected max_count_elements {}",
                max_count_elements
            )));
        }
        let tx = counts
            .into_iter()
            .max_by_key(|&(_, count)| count)
            .map(|(info, _)| info)
            .ok_or(SolClientError::Custom(
                "Can not find the most suitable tx".to_string(),
            ))?;
        Ok(serde_json::from_slice(&tx).unwrap())
    }

    fn deal_get_signatures_for_address_res(
        signatures: HashMap<String, Vec<RpcConfirmedTransactionStatusWithSignature>>,
    ) -> Result<Vec<RpcConfirmedTransactionStatusWithSignature>, SolClientError> {
        let from_num = signatures.len();
        let mut seen = indexmap::IndexMap::new();
        for (_from, sigs) in signatures {
            for sig in sigs {
                let count = seen
                    .entry(serde_json::to_string(&sig).unwrap())
                    .or_insert(0usize);
                *count += 1;
            }
        }

        let signatures = seen
            .into_iter()
            .filter(|(_sig, num)| *num == from_num)
            .map(|(sig, _)| serde_json::from_str(&sig).unwrap())
            .collect::<Vec<_>>();

        Ok(signatures)
    }

    fn is_all_client_available(clients: &[RpcClient]) -> bool {
        let mut all_available = true;
        clients.iter().for_each(|client| {
            if client.get_health().is_err() {
                all_available = false;
                println!("client {} is not available", client.url())
            }
        });
        all_available
    }

    pub(crate) async fn get_program_accounts(
        &self,
        pubkey: Pubkey,
        config: Option<RpcProgramAccountsConfig>,
    ) -> Result<Vec<(Pubkey, Account)>, SolClientError> {
        let mut judge_program_accounts = match &config {
            None => self
                .sol_judge_rpc
                .get_program_accounts(&pubkey)
                .map_err(|err| {
                    println!(
                        "Failed to get program accounts from {}: {:?}",
                        self.sol_judge_rpc.url(),
                        err
                    );
                    SolClientError::Custom(format!("{:?}", err))
                })?,
            Some(config) => self
                .sol_judge_rpc
                .get_program_accounts_with_config(&pubkey, config.clone())
                .map_err(|err| {
                    println!(
                        "Failed to get program accounts from {}: {:?}",
                        self.sol_judge_rpc.url(),
                        err
                    );
                    SolClientError::Custom(format!("{:?}", err))
                })?,
        }
        .into_iter()
        .map(|v| serde_json::to_string(&v).unwrap())
        .collect::<Vec<_>>();

        let (response_sender, mut response_receiver) =
            tokio::sync::mpsc::channel::<ResponseEvent>(1);
        self.task_sender
            .send(TaskEvent::GetProgramAccounts(
                pubkey,
                config,
                response_sender,
            ))
            .await
            .map_err(|err| SolClientError::Internal(err.to_string()))?;

        let extra_res = response_receiver.recv().await;
        if let Some(event) = extra_res {
            match event {
                ResponseEvent::GetProgramAccounts(res) => match res {
                    Ok(accounts) => {
                        judge_program_accounts.extend(
                            accounts
                                .into_iter()
                                .map(|v| serde_json::to_string(&v).unwrap())
                                .collect::<Vec<_>>(),
                        );
                        let mut count = HashMap::new();
                        judge_program_accounts.into_iter().for_each(|v| {
                            count
                                .entry(v)
                                .and_modify(|counter| *counter += 1)
                                .or_insert(1);
                        });
                        Ok(count
                            .into_iter()
                            .filter(|(_k, v)| *v == 2)
                            .map(|(k, _)| serde_json::from_str(&k).unwrap())
                            .collect::<Vec<_>>())
                    }
                    Err(err) => Err(err),
                },
                ResponseEvent::GetTransaction(_) => unreachable!(),
                ResponseEvent::GetSlot(_) => unreachable!(),
                ResponseEvent::GetAccountInfo(_) => unreachable!(),
                ResponseEvent::GetSignaturesForAddress(_) => unreachable!(),
            }
        } else {
            Err(SolClientError::Custom("account not found".to_string()))
        }
    }

    pub(crate) async fn get_signatures_for_address(
        &self,
        pubkey: Pubkey,
        config: Option<GetConfirmedSignaturesForAddress2Config>,
    ) -> Result<Vec<RpcConfirmedTransactionStatusWithSignature>, SolClientError> {
        let mut judge_signatures = match &config {
            None => self
                .sol_judge_rpc
                .get_signatures_for_address(&pubkey)
                .map_err(|err| {
                    println!(
                        "Failed to get signatures_for_address from {}: {:?}",
                        self.sol_judge_rpc.url(),
                        err
                    );
                    SolClientError::Custom(format!("{:?}", err))
                })?,
            Some(config) => self
                .sol_judge_rpc
                .get_signatures_for_address_with_config(
                    &pubkey,
                    GetConfirmedSignaturesForAddress2Config {
                        before: config.before,
                        until: config.until,
                        limit: config.limit,
                        commitment: config.commitment,
                    },
                )
                .map_err(|err| {
                    println!(
                        "Failed to get signatures_for_address from {}: {:?}",
                        self.sol_judge_rpc.url(),
                        err
                    );
                    SolClientError::Custom(format!("{:?}", err))
                })?,
        };

        let (before_signature, first_member) = if judge_signatures.is_empty() {
            return Ok(vec![]);
        } else {
            let first = judge_signatures.remove(0);
            (Signature::from_str(&first.signature).unwrap(), first)
        };

        let mut judge_signatures = judge_signatures
            .into_iter()
            .map(|sig| serde_json::to_string(&sig).unwrap())
            .collect::<Vec<_>>();

        let (response_sender, mut response_receiver) =
            tokio::sync::mpsc::channel::<ResponseEvent>(1);

        let config_to_send = match config {
            None => Some(GetConfirmedSignaturesForAddress2Config {
                before: Some(before_signature),
                ..Default::default()
            }),
            Some(mut c) => {
                c.before = Some(before_signature);
                Some(c)
            }
        };

        self.task_sender
            .send(TaskEvent::GetSignaturesForAddress(
                pubkey,
                config_to_send,
                response_sender,
            ))
            .await
            .map_err(|err| SolClientError::Internal(err.to_string()))?;

        let extra_res = response_receiver.recv().await;
        if let Some(event) = extra_res {
            match event {
                ResponseEvent::GetSignaturesForAddress(signatures_res) => match signatures_res {
                    Ok(sigs) => {
                        let sigs = sigs
                            .into_iter()
                            .map(|sig| serde_json::to_string(&sig).unwrap())
                            .collect::<Vec<_>>();
                        judge_signatures.extend(sigs);
                        let mut check_map = indexmap::IndexMap::new();
                        for sig in judge_signatures {
                            let count = check_map.entry(sig).or_insert(0);
                            *count += 1;
                        }
                        let mut res = check_map
                            .into_iter()
                            .filter(|(_, num)| *num == 2)
                            .map(|(sig, _)| serde_json::from_str(&sig).unwrap())
                            .collect::<Vec<_>>();
                        res.insert(0, first_member);

                        Ok(res)
                    }
                    Err(err) => Err(err),
                },
                ResponseEvent::GetTransaction(_) => unreachable!(),
                ResponseEvent::GetSlot(_) => unreachable!(),
                ResponseEvent::GetAccountInfo(_) => unreachable!(),
                ResponseEvent::GetProgramAccounts(_) => unreachable!(),
            }
        } else {
            Err(SolClientError::Custom("signatures not found".to_string()))
        }
    }

    pub(crate) async fn get_account_info(
        &self,
        pubkey: Pubkey,
        config: Option<RpcAccountInfoConfig>,
    ) -> Result<Option<Account>, SolClientError> {
        let judge_account = match &config {
            None => match self.sol_judge_rpc.get_account(&pubkey) {
                Ok(account) => Some(account),
                Err(err) => {
                    println!(
                        "Failed to get account_info from {}: {:?}",
                        self.sol_judge_rpc.url(),
                        err
                    );
                    return Err(SolClientError::Custom(format!("{:?}", err)));
                }
            },
            Some(config) => {
                match self
                    .sol_judge_rpc
                    .get_account_with_config(&pubkey, config.clone())
                {
                    Ok(res) => res.value,
                    Err(err) => {
                        println!(
                            "Failed to get account_info from {}: {:?}",
                            self.sol_judge_rpc.url(),
                            err
                        );
                        return Err(SolClientError::Custom(format!("{:?}", err)));
                    }
                }
            }
        };

        let (response_sender, mut response_receiver) =
            tokio::sync::mpsc::channel::<ResponseEvent>(1);
        self.task_sender
            .send(TaskEvent::GetAccountInfo(pubkey, config, response_sender))
            .await
            .map_err(|err| SolClientError::Internal(err.to_string()))?;

        let extra_res = response_receiver.recv().await;
        if let Some(event) = extra_res {
            match event {
                ResponseEvent::GetAccountInfo(account_res) => match account_res {
                    Ok(account) => {
                        if account.eq(&judge_account) {
                            Ok(account)
                        } else {
                            Err(SolClientError::Custom(
                                "account is not same with judge".to_string(),
                            ))
                        }
                    }
                    Err(err) => Err(err),
                },
                ResponseEvent::GetTransaction(_) => unreachable!(),
                ResponseEvent::GetSlot(_) => unreachable!(),
                ResponseEvent::GetSignaturesForAddress(_) => unreachable!(),
                ResponseEvent::GetProgramAccounts(_) => unreachable!(),
            }
        } else {
            Err(SolClientError::Custom("account not found".to_string()))
        }
    }

    pub(crate) async fn get_slot(
        &self,
        commitment_config: Option<CommitmentConfig>,
    ) -> Result<u64, SolClientError> {
        let judge_slot = match commitment_config {
            None => self.sol_judge_rpc.get_slot().map_err(|err| {
                println!(
                    "Failed to get slot from {}: {:?}",
                    self.sol_judge_rpc.url(),
                    err
                );
                SolClientError::Custom(format!("{:?}", err))
            })?,
            Some(config) => self
                .sol_judge_rpc
                .get_slot_with_commitment(config)
                .map_err(|err| {
                    println!(
                        "Failed to get slot from {}: {:?}",
                        self.sol_judge_rpc.url(),
                        err
                    );
                    SolClientError::Custom(format!("{:?}", err))
                })?,
        };

        let (response_sender, mut response_receiver) =
            tokio::sync::mpsc::channel::<ResponseEvent>(1);
        self.task_sender
            .send(TaskEvent::GetSlot(commitment_config, response_sender))
            .await
            .map_err(|err| SolClientError::Internal(err.to_string()))?;

        let extra_res = response_receiver.recv().await;
        if let Some(event) = extra_res {
            match event {
                ResponseEvent::GetSlot(slot_res) => match slot_res {
                    Ok(slot) => {
                        if judge_slot <= slot {
                            Ok(judge_slot)
                        } else {
                            Err(SolClientError::Custom(format!(
                                "slot {} is not same with judge {}",
                                slot, judge_slot
                            )))
                        }
                    }
                    Err(err) => Err(err),
                },
                ResponseEvent::GetTransaction(_) => unreachable!(),
                ResponseEvent::GetAccountInfo(_) => unreachable!(),
                ResponseEvent::GetSignaturesForAddress(_) => unreachable!(),
                ResponseEvent::GetProgramAccounts(_) => unreachable!(),
            }
        } else {
            Err(SolClientError::Custom("slot not found".to_string()))
        }
    }

    pub(crate) async fn get_transaction(
        &self,
        signature: Signature,
        tx_config: Option<RpcTransactionConfig>,
    ) -> Result<EncodedConfirmedTransactionWithStatusMeta, SolClientError> {
        let judge_tx = match tx_config {
            None => self
                .sol_judge_rpc
                .get_transaction(&signature, UiTransactionEncoding::Json)
                .map_err(|err| {
                    println!(
                        "Failed to get transaction from {}: {:?}",
                        self.sol_judge_rpc.url(),
                        err
                    );
                    SolClientError::Custom(format!("{:?}", err))
                })?,
            Some(config) => self
                .sol_judge_rpc
                .get_transaction_with_config(&signature, config)
                .map_err(|err| {
                    println!(
                        "Failed to get transaction from {}: {:?}",
                        self.sol_judge_rpc.url(),
                        err
                    );
                    SolClientError::Custom(format!("{:?}", err))
                })?,
        };

        let (response_sender, mut response_receiver) =
            tokio::sync::mpsc::channel::<ResponseEvent>(1);

        self.task_sender
            .send(TaskEvent::GetTransaction(
                signature,
                tx_config,
                response_sender,
            ))
            .await
            .map_err(|err| SolClientError::Internal(err.to_string()))?;
        let extra_res = response_receiver.recv().await;
        if let Some(event) = extra_res {
            match event {
                ResponseEvent::GetTransaction(tx_res) => match tx_res {
                    Ok(tx) => {
                        if tx.eq(&judge_tx) {
                            Ok(tx)
                        } else {
                            Err(SolClientError::Custom(format!(
                                "tx {} from extra rpcs is not same as which from judge rpc",
                                signature
                            )))
                        }
                    }
                    Err(err) => Err(err),
                },
                ResponseEvent::GetSlot(_) => unreachable!(),
                ResponseEvent::GetAccountInfo(_) => unreachable!(),
                ResponseEvent::GetSignaturesForAddress(_) => unreachable!(),
                ResponseEvent::GetProgramAccounts(_) => unreachable!(),
            }
        } else {
            Err(SolClientError::Custom(format!(
                "tx {} not found",
                signature
            )))
        }
    }
}
