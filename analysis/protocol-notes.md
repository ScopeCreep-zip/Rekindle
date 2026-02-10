# Protocol Verification Notes

## Confirmed from Binary Analysis

### Handshake
- **UA01** string confirmed at offset `0x002929c8` in `.rdata` section
- Sent as 4 ASCII bytes on TCP connect: `0x55 0x41 0x30 0x31`

### Authentication
- **UltimateArena** salt confirmed at offset `0x0029b4b4`
- Referenced from function at `0x00637066` (auth hash computation)
- SHA1 is **statically compiled** — no DLL import, no string reference to "SHA1"
- Auth scheme: `SHA1(SHA1(username + password + "UltimateArena") + server_salt)`

### Server Connection
- Primary server: `cs.xf1re.com` (was `cs.xfire.com`, port 25999)
- TCP via WS2_32.dll `connect()` + `send()` + `recv()`
- Async I/O via `WSAAsyncSelect`

### Packet Format
Confirmed by `Buffer::` methods:
- `Buffer::GetHashValueByte()` — type tag for byte values
- `Buffer::GetHashValueInt32()` — type tag for 32-bit integers
- `Buffer::GetHashValueInt64()` — type tag for 64-bit integers
- `Buffer::GetHashValueString()` — type tag for string values
- `Buffer::GetHashValueSessionID()` — type tag for session IDs (128-bit)
- `Buffer::GetHashValueGenericID()` — type tag for generic IDs
- `Buffer::ReadToByteKeyHashValueByteKeyHash()` — nested hash parsing
- `Buffer::ReadToHashValueHash()` — recursive hash/attribute parsing

This confirms the documented attribute system:
```
Packet: [uint16 size][uint16 type_id][uint8 attr_count][attributes...]
Attribute: [key][type_byte][value]
```

### Message Validation
- `ClientProtocol::IsMessageHeaderValid()` — validates packet header (length, type)
- Error: "got message with invalid length %u"

### Protocol Messages Discovered

#### Login/Session
- `UA_LOGIN_SUCCESS` — login success with SystemStatus
- `Disconnect: duplicate login` — kicked by another session
- `Disconnect: failed passive login` — passive auth failure

#### Friends
- `ClientProtocol::ReadFriends()` — friend list (3 arrays: names, nicknames, ?)
- `ClientProtocol::ReadFriendGroups()` — friend group assignments
- `ClientProtocol::ReadFriendNetworkinfo()` — friend network presence (5 arrays)
- `ClientProtocol::ReadFriendsFavoriteServers()` — friend fav servers
- `ClientProtocol::ReadHalfAddedFriends()` — pending friend requests
- `ClientProtocol::ReadPluginFriendGroups()` — IM plugin friend groups

#### Game Status
- `ClientProtocol::ReadSessionGameStatus()` — 4 arrays: sids, gameids, game_ips, game_ports
- `ClientProtocol::ReadSessionGameClientData()` — custom game data
- `ClientProtocol::ReadSessionStatus()` — online status
- `ClientProtocol::ReadSessionTextStatus()` — text status messages
- `ClientProtocol::ReadSessionVoiceStatus()` — voice chat status

#### Chat Rooms
- Full lifecycle: join, leave, create, save, invite, kick/ban
- Room properties: title, password, visibility, MOTD, voice settings
- Lobby system: join/leave/launch/update status
- Grid voice chat with reflectors

#### Clans
- Members, ranks, permissions, events, news, invitations, favorites

#### Channels
- File sharing channels with update notifications

#### Search
- `ClientProtocol::ReadSearchUser()` — user search by name

#### Screenshots/Videos
- Upload, browse, contest support

#### Updates
- Client update protocol with versioned file lists

## P2P Subsystem

### Architecture
```
P2P::Initialize()        — creates UDP socket
P2P::ConnectedToServer() — receives sessionId + downloadId
P2P::SendRequestConnect() — initiates P2P connection
P2P::OnRequestConnect()  — handles incoming P2P requests
P2P::OnNatServerReply()  — NAT traversal server response
P2P::Disconnect()        — teardown
```

### NAT Traversal
- UPnP port mapping (`WANIPConnection`, `WANPPPConnection`)
- NAT type detection via server-mediated checks
- Symmetric sequential port prediction for difficult NATs
- Connection states: originator, recipient, re-handshake
- Salt-based moniker generation for connection authentication

### P2P Connection Lifecycle
1. `SendRequestConnect` with IP, port, localIP, localPort, natType
2. NAT server mediates if needed
3. `HandshakePing` / `HandshakePong` UDP exchange
4. Handshake complete → data transfer
5. Keepalive with adaptive RTO (Retransmission Timeout)
6. SRTT / SERR / RTT tracking for congestion control

### P2P Peer Types
- Chat (1:1 messaging P2P)
- Voice (voice chat)
- File transfer (DLPlugin)
- Net tool (game traffic routing?)

### NAT Assessment
```
UDP consistent translation (port-forwarded): YES+   (GREAT for p2p)
UDP consistent translation:                  YES    (GOOD for p2p)
UDP consistent translation:                  MAYBE  (OK for p2p)
UDP consistent translation:                  NO     (BAD for p2p)
UDP unsolicited messages filtered:           YES    (GOOD for security)
UDP unsolicited messages filtered:           NO     (BAD for security)
```

## Voice Chat

- UDP-based with `xfcodec.dll` / `xfcodec64.dll`
- Grid-based room voice with server-assigned tokens
- Microphone mute/unmute states
- `XFDSBuffer::VoiceChatDataWrite()` — circular buffer for voice data
- Max frame size enforcement

## Game Detection

- `ToucanGetProcessList` / `ToucanUpdateProcessList` — process enumeration
- `XfireProcessList` — shared memory IPC for game process tracking
- `CreateToolhelp32Snapshot` — Win32 process snapshot API
- `EnumProcesses` / `EnumProcessModules` — PSAPI process enumeration
- Per-game config in `xfire_games.ini` with registry keys and process names
- `InGameRenderer` types: D3D8, D3D9, OpenGL (determines overlay hook method)

## IM Plugin Architecture

External IM bridges:
- **AIM/ICQ** — OSCAR/TOC protocol, BART (buddy art) support
- **Yahoo Messenger** — Token-based OAuth via `login.yahoo.com`
- **Google Talk** — XMPP via Google ClientAuth
- **Facebook Chat** — XMPP to `chat.facebook.com`

Each plugin has:
- Login/auth flow
- Contact list sync
- Message send/receive
- Presence status mapping
