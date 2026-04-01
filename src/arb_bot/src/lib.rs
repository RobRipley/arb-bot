use candid::{CandidType, Deserialize};
use ic_cdk_macros::{init, post_upgrade, pre_upgrade, query};

mod state;
mod prices;
mod swaps;

use state::BotConfig;

#[derive(CandidType, Deserialize)]
pub struct InitArgs {
    pub config: BotConfig,
}

#[init]
fn init(args: InitArgs) {
    state::init_state(state::BotState {
        config: args.config,
        ..Default::default()
    });
}

#[pre_upgrade]
fn pre_upgrade() {
    state::save_to_stable_memory();
}

#[post_upgrade]
fn post_upgrade() {
    state::load_from_stable_memory();
}

#[query]
fn get_config() -> BotConfig {
    state::read_state(|s| s.config.clone())
}
