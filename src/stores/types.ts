export interface GameInfo {
  gameName: string;
  gameId: number | null;
  startedAt: number | null;
  serverAddress: string | null;
}

export interface AuditLogEntryDto {
  action: string;
  actorPseudonym: string;
  target: string | null;
  details: string | null;
  timestamp: number;
}

export interface InviteDto {
  code: string;
  createdBy: string;
  maxUses: number | null;
  uses: number;
  expiresAt: number | null;
  createdAt: number;
}
