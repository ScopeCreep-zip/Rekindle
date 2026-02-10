# Xfire.exe Strings Analysis Report

**Binary:** `Xfire.exe` (3,560,832 bytes, PE32 native Win32 C++)
**Build:** Release155b TitanStrike, Version 13133

## Protocol Indicators

### Handshake
- `UA01` — 4-byte ASCII handshake sent on TCP connect (confirmed at offset 0x002929c8)
- `UltimateArena` — auth salt constant (confirmed at offset 0x0029b4b4, xref from `fcn.00637066`)

### Server Addresses (Xf1re patched)
```
cs.xf1re.com           — main chat server (was cs.xfire.com)
client.xf1re.com       — client services
secure.xf1re.com       — secure/auth endpoint
media.xf1re.com        — media services
screenshot.xf1re.com   — screenshot upload
video.xf1re.com        — video services
xf1re.b-cdn.net        — CDN for static assets
```

Additional environments: `dev.corp`, `oc`, `sec`, `stage` subdomains exist.

### Protocol Message Types (UA_ prefixed)
```
UA_LOGIN_SUCCESS
UA_NAT_SEND_USER
UA_CHT_JOIN_ROOM
UA_CHT_LEAVE_ROOM
UA_CHT_SEND_ONE_CLIENT
UA_CHT_SEND_MULTIPLE_CLIENTS
UA_CHT_REQ_CONNECT
UA_CHT_REQ_INVITE
UA_CHT_REQ_INVITE_HANDLED
UA_CHT_REQ_MULTIINVITE
UA_CHT_REQ_ROOM_MESSAGE
UA_CHT_REQ_ROOM_SUMMARY
UA_CHT_REQ_ROOM_DETAIL
UA_CHT_REQ_SAVE_ROOM
UA_CHT_REQ_SET_ROOM_TITLE
UA_CHT_REQ_SET_PASSWORD
UA_CHT_REQ_SET_VISIBILITY
UA_CHT_REQ_SET_MOTD
UA_CHT_REQ_SET_LOBBY
UA_CHT_REQ_LAUNCH_LOBBY
UA_CHT_REQ_JOIN_LOBBY
UA_CHT_REQ_LEAVE_LOBBY
UA_CHT_REQ_UPDATE_LOBBY_LAUNCH_STATUS
UA_CHT_REQ_SET_SILENCED
UA_CHT_REQ_KICKBAN
UA_CHT_REQ_CHANGE_TIER
UA_CHT_REQ_SET_DEFAULT_TIER
UA_CHT_REQ_CHECK_ROOM_AVAILABILITY
UA_CHT_REQ_SET_ROOM_VOICE_SETTINGS
UA_CHT_REQ_SET_SHOW_ENTER_LEAVE_MESSAGES
UA_CHT_REQ_GROUPVOICE_DISCONNECT
UA_CHT_REQ_GROUPVOICE_RESPONSE
UA_CHT_REQ_REFLECTOR_FULL
UA_CHT_REQ_REFLECTOR_READY
UA_CHT_REQ_TEST_P2P_BANDWIDTH_RESPONSE
UA_CHT_REQ_USE_GRID
UA_CHT_USER_LOGON
UA_CHT_USER_LOGOFF
UA_CHT_USER_NET_CONFIG
UA_CH_REQ_FILE
UA_CH_REQ_UPDATES
UA_CH_REQ_NO_UPDATES
UA_CH_SEND_ONE_CLIENT
UA_CH_SEND_MULTIPLE_CLIENTS
UA_CH_USER_LOGON
UA_CH_USER_LOGOFF
```

### ClientProtocol Methods
```
ClientProtocol::IsMessageHeaderValid()
ClientProtocol::ReadFriends()
ClientProtocol::ReadFriendGroups()
ClientProtocol::ReadFriendNetworkinfo()
ClientProtocol::ReadFriendsFavoriteServers()
ClientProtocol::ReadHalfAddedFriends()
ClientProtocol::ReadPluginFriendGroups()
ClientProtocol::ReadGroups()
ClientProtocol::ReadClans()
ClientProtocol::ReadClanMembers()
ClientProtocol::ReadClanPreferences()
ClientProtocol::ReadClanRanks()
ClientProtocol::ReadClanRankPermissions()
ClientProtocol::ReadClanEvents()
ClientProtocol::ReadClanNews()
ClientProtocol::ReadClanInvitations()
ClientProtocol::ReadClanFavoriteServers()
ClientProtocol::ReadUserClanNames()
ClientProtocol::ReadSessionGameStatus()
ClientProtocol::ReadSessionGameClientData()
ClientProtocol::ReadSessionStatus()
ClientProtocol::ReadSessionTextStatus()
ClientProtocol::ReadSessionVoiceStatus()
ClientProtocol::ReadAllServers()
ClientProtocol::ReadGetFavoriteServers()
ClientProtocol::ReadSearchUser()
ClientProtocol::ReadScreenshots()
ClientProtocol::ReadUserScreenshots()
ClientProtocol::ReadUpdate()
ClientProtocol::ReadContests()
ClientProtocol::ReadChannelUpdate()
ClientProtocol::ReadChannelFileInfo()
ClientProtocol::ReadRoomSummary()
ClientProtocol::ReadRoomUsers()
ClientProtocol::ReadIMPlugins()
ClientProtocol::ReadResultPerformQuery()
```

## Buffer/Attribute System
```
Buffer::GetHashValueByte()
Buffer::GetHashValueInt32()
Buffer::GetHashValueInt64()
Buffer::GetHashValueString()
Buffer::GetHashValueSessionID()
Buffer::GetHashValueGenericID()
Buffer::ReadToByteKeyHashValueByteKeyHash()
Buffer::ReadToHashValueHash()
```

This confirms the documented attribute system: typed key-value pairs with byte/int32/int64/string/sessionid/genericid types.

## Network Architecture

### TCP (Primary)
- WS2_32.dll imports: `connect`, `send`, `recv`, `socket`, `bind`, `closesocket`
- Used for main server communication on port 25999

### UDP (P2P)
- Extensive P2P subsystem: `P2P::`, `P2PConnection::`, `P2PNode::`
- NAT traversal with UPnP support
- Symmetric sequential port prediction for NAT punching
- Ping/pong handshake protocol with salt-based authentication
- Connection states: originator, recipient, re-handshake
- Keepalive with adaptive RTO (retransmission timeout)

### HTTP (Web Services)
- WININET.dll for HTTP requests
- Used for: screenshots, videos, authentication to 3rd party (Google, Yahoo, Facebook)

## Voice Chat
```
VoiceChatSubsystemImpl::CreateSocket()
VoiceChatSubsystemImpl::EnableGridVoiceChat()
VoiceChatSubsystemImpl::OnUDPRead()
VoiceChatSubsystemImpl::UpdateGridRoomToken()
```
- UDP-based voice chat
- Grid-based room voice with tokens
- Uses `xfcodec.dll` / `xfcodec64.dll` for audio encoding

## IM Plugin System
Third-party IM integration:
- **AIM** — `AIMPlugin::`, `AIMPluginBARTConnector::`
- **Yahoo** — `YIMPlugin::SendMainLoginAuthPacket()`, Yahoo OAuth token flow
- **Google Talk** — XMPP with `jabber:iq:roster`, Google ClientAuth
- **Facebook Chat** — XMPP to `chat.facebook.com`

## Game Detection
```
ToucanGetProcessList
ToucanUpdateProcessList
XfireProcessList
XfireProcessListMutex
CreateToolhelp32Snapshot
EnumProcessModules
EnumProcesses
XfireIPCSharedMemory-%u
```

Game detection via:
1. Process enumeration (`CreateToolhelp32Snapshot`, `EnumProcesses`)
2. Shared memory IPC (`XfireIPCSharedMemory`)
3. Registry-based launcher detection (per `xfire_games.ini`)
4. `InGameRenderer` types: D3D8, D3D9, OpenGL

## Skin System
```
xmlskinparser.cpp
flatskintree.cpp
skincomponents.cpp
skintreemanager.cpp
skintreenode.cpp
skintrees.cpp
skintreetile.cpp
SkinBlock.cpp
Tiles.cpp
Free2PlaySkinWnd.cpp
```

XML-based skinning engine with:
- Tile-based layout (position, size, z-order, justification)
- Named components mapped to code behaviors
- GIF image assets for all UI chrome
- Theme-based color overrides (RGBA)
- Bitmap references by name
- ZIP-packaged skin distribution

## Xf1re-Specific Strings
```
xf1re.b-cdn.net                    — CDN
http://origin.video.xf1re.com/...  — Video service
http://www.xf1re.com/              — Website
```

The Xf1re team patched all server addresses but kept the original binary intact otherwise. This is the genuine Xfire Release155b codebase with minimal modifications.
