use std::str::FromStr;

use self::io::WebIo;
use self::wallet::BrowserWalletUtils;
use crate::rpc_client::HttpClient;
use crate::utils::to_js_result;
use crate::{
    sdk::masp::WebShieldedUtils,
    utils::{set_panic_hook, to_bytes},
};
use borsh::BorshDeserialize;
use js_sys::Uint8Array;
use namada::ledger::{eth_bridge::bridge_pool::build_bridge_pool_tx, pos::common::SecretKey};
use namada::namada_sdk::masp::ShieldedContext;
use namada::namada_sdk::rpc::query_epoch;
use namada::namada_sdk::signing::{find_key_by_pk, SigningTxData};
use namada::namada_sdk::tx::{
    build_bond, build_ibc_transfer, build_reveal_pk, build_transfer, build_unbond,
    build_vote_proposal, build_withdraw, is_reveal_pk_needed, process_tx,
};
use namada::namada_sdk::wallet::{Store, Wallet};
use namada::namada_sdk::{Namada, NamadaImpl};
use namada::proto::Tx;
use namada::types::address::{masp_tx_key, Address};
use wasm_bindgen::{prelude::wasm_bindgen, JsError, JsValue};
pub mod io;
pub mod masp;
mod signature;
mod tx;
mod wallet;

#[wasm_bindgen]
#[derive(Copy, Clone, Debug)]
pub enum TxType {
    Bond = 1,
    Unbond = 2,
    Withdraw = 3,
    Transfer = 4,
    IBCTransfer = 5,
    EthBridgeTransfer = 6,
    RevealPK = 7,
    VoteProposal = 8,
}

#[wasm_bindgen]
pub struct BuiltTx {
    tx: Tx,
    signing_data: SigningTxData,
}

#[wasm_bindgen]
impl BuiltTx {
    pub fn tx_bytes(&self) -> Result<Vec<u8>, JsError> {
        Ok(borsh::to_vec(&self.tx)?)
    }
}

/// Represents the Sdk public API.
#[wasm_bindgen]
pub struct Sdk {
    namada: NamadaImpl<HttpClient, BrowserWalletUtils, WebShieldedUtils, WebIo>,
}

#[wasm_bindgen]
/// Sdk mostly wraps the logic of the Sdk struct members, making it a part of public API.
/// For more details, navigate to the corresponding modules.
impl Sdk {
    #[wasm_bindgen(constructor)]
    pub fn new(url: String) -> Self {
        set_panic_hook();
        let client: HttpClient = HttpClient::new(url);
        let wallet: Wallet<wallet::BrowserWalletUtils> =
            Wallet::new(BrowserWalletUtils {}, Store::default());
        let shielded_ctx: ShieldedContext<masp::WebShieldedUtils> = ShieldedContext::default();

        let namada = NamadaImpl::native_new(
            client,
            wallet,
            shielded_ctx,
            WebIo,
            //NAM address
            Address::from_str("tnam1qxuqn53dtcckynnm35n8s27cftxcqym7gvesjrp9").unwrap(),
        );

        Sdk { namada }
    }

    pub async fn has_masp_params() -> Result<JsValue, JsValue> {
        let has = has_masp_params().await?;

        Ok(js_sys::Boolean::from(has.as_bool().unwrap()).into())
    }

    pub async fn fetch_and_store_masp_params() -> Result<(), JsValue> {
        fetch_and_store_masp_params().await?;
        Ok(())
    }

    pub async fn load_masp_params(&mut self) -> Result<(), JsValue> {
        let params = get_masp_params().await?;
        let params_iter = js_sys::try_iter(&params)?.ok_or_else(|| "Can't iterate over JsValue")?;
        let mut params_bytes = params_iter.map(|p| to_bytes(p.unwrap()));

        let spend = params_bytes.next().unwrap();
        let output = params_bytes.next().unwrap();
        let convert = params_bytes.next().unwrap();

        // We are making sure that there are no more params left
        assert_eq!(params_bytes.next(), None);

        let mut shielded = self.namada.shielded_mut().await;
        *shielded = WebShieldedUtils::new(spend, output, convert);

        Ok(())
    }

    pub async fn add_spending_key(&mut self, xsk: &str, alias: &str) {
        let mut wallet = self.namada.wallet_mut().await;
        wallet::add_spending_key(&mut wallet, xsk, alias)
    }

    pub async fn sign_tx(
        &mut self,
        built_tx: BuiltTx,
        tx_msg: &[u8],
        private_key: Option<String>,
    ) -> Result<JsValue, JsError> {
        let signing_key = match private_key {
            Some(private_key) => SecretKey::from_str(&format!("{}{}", "00", private_key))?,
            // If no private key is provided, we assume masp source and use the masp_tx_key
            None => masp_tx_key(),
        };

        let args = tx::tx_args_from_slice(tx_msg)?;

        let BuiltTx {
            mut tx,
            signing_data,
        } = built_tx;

        let signing_keys = vec![signing_key.clone()];

        if let Some(account_public_keys_map) = signing_data.account_public_keys_map.clone() {
            // Sign the raw header
            tx.sign_raw(
                signing_keys,
                account_public_keys_map,
                signing_data.owner.clone(),
            );
        }

        // For shielded transactions, we need to sign the wrapper with the fee payer key which
        // is the private key of disposable account. Otherwise we sign with passed signer.
        let key = {
            let mut wallet = self.namada.wallet_mut().await;
            find_key_by_pk(&mut *wallet, &args, &signing_data.fee_payer)
        }
        .unwrap_or(signing_key);

        // Sign the fee header
        tx.sign_wrapper(key);

        to_js_result(borsh::to_vec(&tx)?)
    }

    pub async fn process_tx(&mut self, tx_bytes: &[u8], tx_msg: &[u8]) -> Result<(), JsError> {
        let args = tx::tx_args_from_slice(tx_msg)?;

        let tx = Tx::try_from_slice(tx_bytes)?;
        process_tx(&self.namada, &args, tx).await?;

        Ok(())
    }

    /// Build transaction for specified type, return bytes to client
    pub async fn build_tx(
        &mut self,
        tx_type: TxType,
        specific_msg: &[u8],
        tx_msg: &[u8],
        gas_payer: String,
    ) -> Result<JsValue, JsError> {
        let tx = match tx_type {
            TxType::Bond => {
                self.build_bond(specific_msg, tx_msg, Some(gas_payer))
                    .await?
                    .tx
            }
            TxType::Unbond => {
                self.build_unbond(specific_msg, tx_msg, Some(gas_payer))
                    .await?
                    .tx
            }
            TxType::Withdraw => {
                self.build_withdraw(specific_msg, tx_msg, Some(gas_payer))
                    .await?
                    .tx
            }
            TxType::Transfer => {
                self.build_transfer(specific_msg, tx_msg, None, Some(gas_payer))
                    .await?
                    .tx
            }
            TxType::IBCTransfer => {
                self.build_ibc_transfer(specific_msg, tx_msg, Some(gas_payer))
                    .await?
                    .tx
            }
            TxType::EthBridgeTransfer => {
                self.build_eth_bridge_transfer(specific_msg, tx_msg, Some(gas_payer))
                    .await?
                    .tx
            }
            TxType::RevealPK => self.build_reveal_pk(tx_msg, gas_payer).await?.tx,
            TxType::VoteProposal => {
                self.build_vote_proposal(specific_msg, tx_msg, Some(gas_payer))
                    .await?
                    .tx
            }
        };

        to_js_result(borsh::to_vec(&tx)?)
    }

    // Append signatures and return tx bytes
    pub fn append_signature(
        &self,
        tx_bytes: &[u8],
        sig_msg_bytes: &[u8],
    ) -> Result<JsValue, JsError> {
        let mut tx: Tx = Tx::try_from_slice(tx_bytes)?;
        let signature::SignatureMsg {
            pubkey,
            raw_indices,
            raw_signature,
            wrapper_indices,
            wrapper_signature,
        } = signature::SignatureMsg::try_from_slice(&sig_msg_bytes)?;

        let raw_sig_section =
            signature::construct_signature_section(&pubkey, &raw_indices, &raw_signature, &tx)?;
        tx.add_section(raw_sig_section);

        let wrapper_sig_section = signature::construct_signature_section(
            &pubkey,
            &wrapper_indices,
            &wrapper_signature,
            &tx,
        )?;
        tx.add_section(wrapper_sig_section);

        tx.protocol_filter();

        to_js_result(borsh::to_vec(&tx)?)
    }

    pub async fn build_transfer(
        &mut self,
        transfer_msg: &[u8],
        tx_msg: &[u8],
        xsk: Option<String>,
        _gas_payer: Option<String>,
    ) -> Result<BuiltTx, JsError> {
        let mut args = tx::transfer_tx_args(transfer_msg, tx_msg, xsk)?;
        let (tx, signing_data, _) = build_transfer(&self.namada, &mut args).await?;

        Ok(BuiltTx { tx, signing_data })
    }

    pub async fn build_ibc_transfer(
        &mut self,
        ibc_transfer_msg: &[u8],
        tx_msg: &[u8],
        _gas_payer: Option<String>,
    ) -> Result<BuiltTx, JsError> {
        let args = tx::ibc_transfer_tx_args(ibc_transfer_msg, tx_msg)?;
        let (tx, signing_data, _) = build_ibc_transfer(&self.namada, &args).await?;

        Ok(BuiltTx { tx, signing_data })
    }

    pub async fn build_eth_bridge_transfer(
        &mut self,
        eth_bridge_transfer_msg: &[u8],
        tx_msg: &[u8],
        _gas_payer: Option<String>,
    ) -> Result<BuiltTx, JsError> {
        let args = tx::eth_bridge_transfer_tx_args(eth_bridge_transfer_msg, tx_msg)?;
        let (tx, signing_data, _) = build_bridge_pool_tx(&self.namada, args.clone()).await?;

        Ok(BuiltTx { tx, signing_data })
    }

    pub async fn build_vote_proposal(
        &mut self,
        vote_proposal_msg: &[u8],
        tx_msg: &[u8],
        _gas_payer: Option<String>,
    ) -> Result<BuiltTx, JsError> {
        let args = tx::vote_proposal_tx_args(vote_proposal_msg, tx_msg)?;
        let epoch = query_epoch(self.namada.client()).await?;

        let (tx, signing_data, _) = build_vote_proposal(&self.namada, &args, epoch)
            .await
            .map_err(JsError::from)?;

        Ok(BuiltTx { tx, signing_data })
    }

    pub async fn build_bond(
        &mut self,
        bond_msg: &[u8],
        tx_msg: &[u8],
        _gas_payer: Option<String>,
    ) -> Result<BuiltTx, JsError> {
        let args = tx::bond_tx_args(bond_msg, tx_msg)?;
        let (tx, signing_data, _) = build_bond(&self.namada, &args).await?;

        Ok(BuiltTx { tx, signing_data })
    }

    pub async fn build_unbond(
        &mut self,
        unbond_msg: &[u8],
        tx_msg: &[u8],
        _gas_payer: Option<String>,
    ) -> Result<BuiltTx, JsError> {
        let args = tx::unbond_tx_args(unbond_msg, tx_msg)?;
        let (tx, signing_data, _, _) = build_unbond(&self.namada, &args).await?;

        Ok(BuiltTx { tx, signing_data })
    }

    pub async fn build_withdraw(
        &mut self,
        withdraw_msg: &[u8],
        tx_msg: &[u8],
        _gas_payer: Option<String>,
    ) -> Result<BuiltTx, JsError> {
        let args = tx::withdraw_tx_args(withdraw_msg, tx_msg)?;
        let (tx, signing_data, _) = build_withdraw(&self.namada, &args).await?;

        Ok(BuiltTx { tx, signing_data })
    }

    pub async fn build_reveal_pk(
        &mut self,
        tx_msg: &[u8],
        _gas_payer: String,
    ) -> Result<BuiltTx, JsError> {
        let args = tx::tx_args_from_slice(tx_msg)?;
        let public_key = args.signing_keys[0].clone();
        let (tx, signing_data, _) =
            build_reveal_pk(&self.namada, &args.clone(), &public_key).await?;

        Ok(BuiltTx { tx, signing_data })
    }

    // Helper function to reveal public key
    pub async fn reveal_pk(&mut self, signing_key: String, tx_msg: &[u8]) -> Result<(), JsError> {
        let args = tx::tx_args_from_slice(tx_msg)?;
        let pk = &args
            .signing_keys
            .clone()
            .into_iter()
            .nth(0)
            .expect("No public key provided");
        let address = Address::from(pk);

        if is_reveal_pk_needed(self.namada.client(), &address, false).await? {
            let built_tx = self.build_reveal_pk(tx_msg, String::from("")).await?;
            // Conversion from JsValue so we can use self.sign_tx
            let tx_bytes =
                Uint8Array::new(&self.sign_tx(built_tx, tx_msg, Some(signing_key)).await?).to_vec();
            self.process_tx(&tx_bytes, tx_msg).await?;
        }

        Ok(())
    }
}

#[wasm_bindgen(module = "/src/sdk/mod.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = "getMaspParams")]
    async fn get_masp_params() -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = "hasMaspParams")]
    async fn has_masp_params() -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = "fetchAndStoreMaspParams")]
    async fn fetch_and_store_masp_params() -> Result<JsValue, JsValue>;
}
