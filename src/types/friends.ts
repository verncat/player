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
}

export interface FriendMessagePage {
  messages: FriendMessage[];
  has_more: boolean;
}
