// Optional official Discord Social SDK adapter. All calls run on one Rust-owned thread.
#define DISCORDPP_IMPLEMENTATION
#include "discordpp.h"
#include <algorithm>
#include <cstring>
#include <string>

struct Social {
    discordpp::Client client;
    std::string error;
};
static std::string json(std::string const& text) {
    std::string out = "\"";
    static char const* hex = "0123456789abcdef";
    for (unsigned char c : text) {
        if (c == '"' || c == '\\') { out += '\\'; out += c; }
        else if (c < 32) { out += "\\u00"; out += hex[c >> 4]; out += hex[c & 15]; }
        else out += c;
    }
    return out + '"';
}
extern "C" {
void* player_social_create(uint64_t app_id) noexcept {
    try {
        auto social = new Social;
        social->client.SetApplicationId(app_id);
        auto proof = social->client.CreateAuthorizationCodeVerifier();
        discordpp::AuthorizationArgs args;
        args.SetClientId(app_id);
        args.SetScopes(discordpp::Client::GetDefaultPresenceScopes());
        args.SetCodeChallenge(proof.Challenge());
        social->client.Authorize(args, [social, app_id, proof](auto result, std::string code, std::string redirect) {
            if (!result.Successful()) { social->error = "Discord sign-in was cancelled or failed."; return; }
            social->client.GetToken(app_id, code, proof.Verifier(), redirect,
                [social](auto result, std::string token, std::string, auto type, int32_t, std::string) {
                    if (!result.Successful()) { social->error = "Discord token exchange failed."; return; }
                    // Access tokens stay in memory. Reconnect explicitly after restarting Player.
                    social->client.UpdateToken(type, token, [social](auto result) {
                        if (result.Successful()) social->client.Connect();
                        else social->error = "Discord authentication failed.";
                    });
                });
        });
        return social;
    } catch (...) { return nullptr; }
}
void player_social_destroy(void* ptr) noexcept { try { delete static_cast<Social*>(ptr); } catch (...) {} }
void player_social_presence(void* ptr, char const* title, char const* artist, char const* album, char const* url, bool playing, uint64_t start, uint64_t end) noexcept {
    try {
        auto& client = static_cast<Social*>(ptr)->client;
        if (!playing) { client.ClearRichPresence(); return; }
        discordpp::Activity activity;
        if (playing) {
            activity.SetType(discordpp::ActivityTypes::Listening);
            activity.SetDetails(std::string(title));
            activity.SetState(std::string(artist));
            discordpp::ActivityAssets assets;
            assets.SetLargeImage("player");
            if (*album) assets.SetLargeText(std::string(album));
            if (*url) { assets.SetSmallImage("player"); assets.SetSmallUrl(std::string(url)); }
            activity.SetAssets(assets);
            discordpp::ActivityTimestamps timestamps;
            timestamps.SetStart(start);
            if (end) timestamps.SetEnd(end);
            activity.SetTimestamps(timestamps);
        }
        client.UpdateRichPresence(activity, [](auto) {});
    } catch (...) {}
}
size_t player_social_poll(void* ptr, char* buffer, size_t capacity) noexcept {
    try {
        discordpp::RunCallbacks();
        auto& social = *static_cast<Social*>(ptr);
        bool ready = social.client.GetStatus() == discordpp::Client::Status::Ready;
        auto output = std::string("{\"connected\":") + (ready ? "true" : "false") + ",\"error\":" + json(social.error) + ",\"users\":[";
        bool first = true;
        if (ready) for (auto const& relationship : social.client.GetRelationships()) {
            if (relationship.DiscordRelationshipType() != discordpp::RelationshipType::Friend) continue;
            auto user = relationship.User();
            if (!user) continue;
            if (!first) output += ',';
            first = false;
            output += "{\"id\":" + json(std::to_string(user->Id())) + ",\"name\":" + json(user->DisplayName());
            output += ",\"avatar\":" + json(user->AvatarUrl(discordpp::UserHandle::AvatarType::Png, discordpp::UserHandle::AvatarType::Png));
            std::string url, title, artist, album;
            if (auto activity = user->GameActivity()) {
                // Ignore non-listening activities belonging to the same application.
                if (activity->Type() == discordpp::ActivityTypes::Listening) {
                    title = activity->Details().value_or(""); artist = activity->State().value_or("");
                    if (auto assets = activity->Assets()) { url = assets->SmallUrl().value_or(""); album = assets->LargeText().value_or(""); }
                }
            }
            output += ",\"url\":" + json(url) + ",\"title\":" + json(title) + ",\"artist\":" + json(artist) + ",\"album\":" + json(album) + '}';
        }
        output += "]}";
        if (output.size() >= capacity) return 0;
        std::memcpy(buffer, output.data(), output.size());
        return output.size();
    } catch (...) { return 0; }
}
}
