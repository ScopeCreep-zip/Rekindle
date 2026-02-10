# Xfire Binary Protocol Specification

Reference compiled from OpenFire docs (Iain McGinniss, 2007), gfire source, PFire source,
and IMFreedom Xfire KB.

## Connection

| Field       | Value                          |
|-------------|--------------------------------|
| Server      | `cs.xfire.com`                 |
| Port        | 25999 (TCP)                    |
| P2P         | UDP (negotiated ports)         |
| Byte order  | Little-endian                  |
| Strings     | UTF-8 (attr names: ISO 8859-1)|
| Handshake   | Client sends `"UA01"` (4 bytes)|

## Packet Structure

```
Offset  Size     Field
0x00    2 bytes  Message size (uint16 LE, includes header)
0x02    2 bytes  Message type ID (uint16 LE)
0x04    1 byte   Number of attributes (uint8)
0x05    ...      Attribute data (variable length)
```

## Attribute Encoding

Each attribute:
```
[1 byte: name_length]
[N bytes: attribute_name (ISO 8859-1)]
[1 byte: value_type_id]
[variable: value_data]
```

### Value Types

| ID   | Type               | Encoding                                             |
|------|--------------------|------------------------------------------------------|
| 0x01 | String             | uint16 length + UTF-8 data                           |
| 0x02 | Int32              | 4 bytes, little-endian                               |
| 0x03 | Session ID (SID)   | 16 bytes raw                                         |
| 0x04 | List               | 1 byte item_type + uint16 count + items              |
| 0x05 | String-keyed Map   | 1 byte count + entries (string key + typed value)    |
| 0x06 | DID                | 16 bytes raw (purpose unclear)                       |
| 0x09 | Int-keyed Map      | 1 byte count + entries (uint8 key + typed value)     |

## Authentication Flow

```
1. Client -> Server:  TCP connect, send "UA01"
2. Client -> Server:  Packet 18 (ClientInfo: skin, version, protocol_version)
3. Client -> Server:  Packet 3  (ClientVersion: version=67 as uint32)
4. Server -> Client:  Packet 128 (LoginChallenge: 40-byte salt)
5. Client -> Server:  Packet 1  (LoginRequest: username + hashed_password)
6. Server -> Client:  Packet 130 (LoginSuccess) or Packet 129 (LoginFailure)
```

### Password Hashing

```
step1 = SHA1(username + password + "UltimateArena")   // stored in client config
step2 = SHA1(step1_bytes + server_salt_40_bytes)       // sent over wire
```

The constant `"UltimateArena"` is the original company name (Ultimate Arena, Inc.).

## Packet Types - Client to Server

| ID  | Hex  | Name                    | Key Attributes                              |
|-----|------|-------------------------|---------------------------------------------|
| 1   | 0x01 | LoginRequest            | `name` (str), `password` (str, hashed)      |
| 2   | 0x02 | ChatMessage             | `sid` (SID), `peermsg` (map)                |
| 3   | 0x03 | ClientVersion           | `version` (int32)                           |
| 4   | 0x04 | GameStatus              | `gameid` (int32), `gip` (int32), `gport`    |
| 5   | 0x05 | FriendsOfFriendRequest  | `sid` (list of SIDs)                        |
| 6   | 0x06 | AddFriend               | `name` (str), `msg` (str)                   |
| 7   | 0x07 | AcceptFriendRequest     | `name` (str)                                |
| 8   | 0x08 | RejectFriendRequest     | `name` (str)                                |
| 9   | 0x09 | RemoveFriend            | `userid` (int32)                            |
| 10  | 0x0A | UserLookup              | `name` (str)                                |
| 11  | 0x0B | SetStatus               | `status` (int32), `msg` (str)               |
| 12  | 0x0C | KeepAlive               | `value` (int32) - echoes server value       |
| 14  | 0x0E | ChangeNickname          | `nick` (str)                                |
| 16  | 0x10 | ClientConfiguration     | Various config attributes                   |
| 18  | 0x12 | ClientInfo              | `skin` (str), `version` (int32)             |

## Packet Types - Server to Client

| ID  | Hex  | Name                    | Key Attributes                              |
|-----|------|-------------------------|---------------------------------------------|
| 128 | 0x80 | LoginChallenge          | `salt` (str, 40 bytes)                      |
| 129 | 0x81 | LoginFailure            | `reason` (int32)                            |
| 130 | 0x82 | LoginSuccess            | `userid` (int32), `sid` (SID), `nick` (str) |
| 131 | 0x83 | FriendList              | `userid` (list), `name` (list), `nick` (list)|
| 132 | 0x84 | SessionAssign           | `userid` (list), `sid` (list)               |
| 133 | 0x85 | ChatMessage             | `sid` (SID), `peermsg` (map)                |
| 134 | 0x86 | NewVersionAvailable     | `version` (int32), `flags` (list)           |
| 135 | 0x87 | FriendGameInfo          | `userid` (list), `gameid` (list), `gip`, `gport`|
| 136 | 0x88 | FriendsOfFriends        | `userid` (list), `sid` (list of SID lists)  |
| 137 | 0x89 | InvitationSent          | `name` (str)                                |
| 138 | 0x8A | IncomingInvitation      | `name` (str), `nick` (str), `msg` (str)     |
| 139 | 0x8B | UserSearchResults       | `userid` (list), `name` (list), `nick` (list)|
| 140 | 0x8C | FriendVoipInfo          | `userid` (list), codec/IP/port attributes   |
| 141 | 0x8D | FriendStatusMessage     | `userid` (list), `msg` (list)               |
| 142 | 0x8E | FriendStatusChange      | `userid` (list), `status` (list)            |
| 143 | 0x8F | BuddyAddRequest         | `userid` (list), `name` (list)              |
| 154 | 0x9A | GroupList               | `groupid` (list), `name` (list)             |
| 158 | 0x9E | GroupMemberAssign       | `userid` (list), `groupid` (list)           |
| 400 |      | DID Message             | `did` (DID) - purpose unknown               |

## Chat Message Subtypes (peermsg map)

The `peermsg` attribute in Packets 2/133 is a string-keyed map containing:

| Key       | Value | Meaning                                              |
|-----------|-------|------------------------------------------------------|
| `msgtype` | 0     | Content message (`imindex` + `im` text)              |
| `msgtype` | 1     | Acknowledgement (`imindex` echoed back)              |
| `msgtype` | 2     | Client info exchange (IP, port, localIP, localPort)  |
| `msgtype` | 3     | Typing notification (`imindex`, `typing` flag)       |

## P2P Communication

- Uses UDP with negotiated ports
- NAT traversal via hole-punching
- Fallback to server-relayed messaging if P2P fails
- P2P channels used for: direct IM, file transfer, voice chat

## Game Detection

The client uses two mechanisms:
1. **Process scanning** - periodically enumerate running processes, match against known game list
2. **LSP (Layered Service Provider)** - Windows network hook to detect which game server
   the user is connected to (captures destination IP:port of game traffic)

The game list is downloaded from the server and maps process names to game IDs.

## Key Constants

| Constant          | Value                    | Usage                        |
|-------------------|--------------------------|------------------------------|
| `"UA01"`          | 0x55 0x41 0x30 0x31     | Connection handshake         |
| `"UltimateArena"` | ASCII string             | Password hash salt constant  |
| Port 25999        | TCP                      | Main server port             |
| Client v1.127     | Last known version       | PFire server targets this    |
