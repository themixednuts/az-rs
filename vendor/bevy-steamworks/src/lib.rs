use std::{ops::Deref, sync::Mutex};

use bevy_app::{App, First, Plugin};
use bevy_ecs::{
    message::Message,
    prelude::{MessageWriter, Resource},
    schedule::{IntoScheduleConfigs, SystemSet},
    system::Res,
};

pub use steamworks::{
    networking_messages, networking_sockets, networking_types, networking_utils,
    restart_app_if_necessary, stats, AccountId, AppIDs, AppId, Apps, AuthSessionError,
    AuthSessionTicketResponse, AuthSessionValidateError, AuthTicket, Callback, CallbackHandle,
    CallbackResult, ChatMemberStateChange, ComparisonFilter, CreateQueryError, DistanceFilter,
    DownloadItemResult, FileType, FloatingGamepadTextInputDismissed, FloatingGamepadTextInputMode,
    Friend, FriendFlags, FriendGame, FriendState, Friends, GameId, GameLobbyJoinRequested,
    GameOverlayActivated, GamepadTextInputDismissed, GamepadTextInputLineMode,
    GamepadTextInputMode, Input, InstallInfo, InvalidErrorCode, ItemState, Leaderboard,
    LeaderboardDataRequest, LeaderboardDisplayType, LeaderboardEntry, LeaderboardScoreUploaded,
    LeaderboardSortMethod, LobbyChatUpdate, LobbyDataUpdate, LobbyId, LobbyKey,
    LobbyKeyTooLongError, LobbyListFilter, LobbyType, Matchmaking, MicroTxnAuthorizationResponse,
    NearFilter, NearFilters, Networking, NotificationPosition, NumberFilter, NumberFilters,
    OverlayToStoreFlag, P2PSessionConnectFail, P2PSessionRequest, PersonaChange,
    PersonaStateChange, PublishedFileId, PublishedFileVisibility, QueryHandle, QueryResult,
    QueryResults, RemotePlay, RemotePlayConnected, RemotePlayDisconnected, RemotePlaySession,
    RemotePlaySessionId, RemoteStorage, SIResult, SResult, SendType, Server, ServerMode,
    SteamAPIInitError, SteamDeviceFormFactor, SteamError, SteamFile, SteamFileInfo,
    SteamFileReader, SteamFileWriter, SteamId, SteamServerConnectFailure, SteamServersConnected,
    SteamServersDisconnected, StringFilter, StringFilterKind, StringFilters,
    TicketForWebApiResponse, UGCContentDescriptorID, UGCQueryType, UGCStatisticType, UGCType,
    UpdateHandle, UpdateStatus, UpdateWatchHandle, UploadScoreMethod, User, UserAchievementStored,
    UserList, UserListOrder, UserStats, UserStatsReceived, UserStatsStored, Utils,
    ValidateAuthTicketResponse, RESULTS_PER_PAGE, UGC,
};

#[derive(Message, Debug)]
#[allow(missing_docs)]
pub enum SteamworksEvent {
    CallbackResult(CallbackResult),
}

#[derive(Resource, Clone)]
pub struct Client(steamworks::Client);

impl Deref for Client {
    type Target = steamworks::Client;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct SteamworksPlugin {
    steam: Mutex<Option<steamworks::Client>>,
}

impl SteamworksPlugin {
    pub fn init_app(app_id: impl Into<AppId>) -> Result<Self, SteamAPIInitError> {
        Ok(Self {
            steam: Mutex::new(Some(steamworks::Client::init_app(app_id.into())?)),
        })
    }

    pub fn init() -> Result<Self, SteamAPIInitError> {
        Ok(Self {
            steam: Mutex::new(Some(steamworks::Client::init()?)),
        })
    }
}

impl From<steamworks::Client> for SteamworksPlugin {
    fn from(client: steamworks::Client) -> Self {
        Self {
            steam: Mutex::new(Some(client)),
        }
    }
}

impl Plugin for SteamworksPlugin {
    fn build(&self, app: &mut App) {
        let client = self
            .steam
            .lock()
            .expect("steamworks plugin mutex should not be poisoned")
            .take()
            .expect("SteamworksPlugin was initialized more than once");

        app.insert_resource(Client(client.clone()))
            .add_message::<SteamworksEvent>()
            .configure_sets(First, SteamworksSystem::RunCallbacks)
            .add_systems(
                First,
                run_steam_callbacks
                    .in_set(SteamworksSystem::RunCallbacks)
                    .before(bevy_ecs::message::MessageUpdateSystems),
            );
    }
}

#[derive(Debug, Clone, Copy, Eq, Hash, SystemSet, PartialEq)]
pub enum SteamworksSystem {
    RunCallbacks,
}

fn run_steam_callbacks(client: Res<Client>, mut output: MessageWriter<SteamworksEvent>) {
    client.process_callbacks(|callback| {
        output.write(SteamworksEvent::CallbackResult(callback));
    });
}
