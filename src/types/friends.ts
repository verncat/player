export interface FriendProfile {
  profile_id: string;
  name: string;
  soulseek_users: string[];
  resolved_at: number | null;
}

export interface FriendChat {
  friend: FriendProfile;
  account: string;
  usernames: string[];
  enabled: boolean;
  configured: boolean;
}

export interface FriendMessage {
  id: number;
  direction: 'incoming' | 'outgoing';
  body: string;
  timestamp: number;
  state: 'received' | 'sending' | 'submitted' | 'unknown' | 'failed';
  track_request: { id: string; title: string; artist: string; album: string; incoming: boolean; state: string; error: string } | null;
}

export interface FriendMessagePage {
  messages: FriendMessage[];
  has_more: boolean;
}
