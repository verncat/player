// Official Social SDK reader. No calls to UpdateRichPresence or SetOnlineStatus.
// Reference: https://discord.com/developers/docs/social-sdk/classdiscordpp_1_1UserHandle.html
#define DISCORDPP_IMPLEMENTATION
#include "discordpp.h"
#include <chrono>
#include <cstdint>
#include <iostream>
#include <string>
#include <thread>

int main(int argc, char** argv) {
    if (argc != 3) return 1;
    const auto app_id = std::stoull(argv[1]);
    const auto timeout = std::chrono::seconds(std::stoi(argv[2]));
    discordpp::Client client;
    client.SetApplicationId(app_id);
    std::string failure;
    auto proof = client.CreateAuthorizationCodeVerifier();
    discordpp::AuthorizationArgs login;
    login.SetClientId(app_id);
    login.SetScopes(discordpp::Client::GetDefaultPresenceScopes());
    login.SetCodeChallenge(proof.Challenge());
    std::cerr << "Approve the Social SDK sign-in in Discord or your browser.\n";
    client.Authorize(login, [&, proof](discordpp::ClientResult result, std::string code, std::string redirect) {
        if (!result.Successful()) {
            failure = "Sign-in failed: " + result.Error() + " (code " +
                std::to_string(result.ErrorCode()) + "). Check Public Client and the OAuth2 redirect.";
            return;
        }
        client.GetToken(app_id, code, proof.Verifier(), redirect,
            [&](discordpp::ClientResult exchanged, std::string token, std::string,
                discordpp::AuthorizationTokenType type, int32_t, std::string) {
                if (!exchanged.Successful()) {
                    failure = "OAuth token exchange failed. Check Public Client and redirect settings.";
                    return;
                }
                // Tokens remain in memory and are never printed or saved by this reader.
                client.UpdateToken(type, token, [&](discordpp::ClientResult updated) {
                    if (updated.Successful()) client.Connect();
                    else failure = "The Social SDK could not authenticate.";
                });
            });
    });

    const auto deadline = std::chrono::steady_clock::now() + timeout;
    bool ready = false;
    bool saw_activity = false;
    while (std::chrono::steady_clock::now() < deadline) {
        discordpp::RunCallbacks();
        if (!failure.empty()) {
            std::cerr << failure << '\n';
            return 1;
        }
        if (client.GetStatus() == discordpp::Client::Status::Ready) {
            if (!ready) {
                std::cerr << "Reading the existing Player activity. Keep playback and Discord discovery enabled.\n";
                ready = true;
            }
            if (auto user = client.GetCurrentUserV2()) {
                if (auto activity = user->GameActivity()) {
                    saw_activity = true;
                    if (auto assets = activity->Assets()) {
                        if (auto url = assets->SmallUrl()) {
                            // The value returned by Discord, not a URL constructed from local settings.
                            std::cout << *url << std::endl;
                            return 0;
                        }
                    }
                }
            }
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(50));
    }
    if (!ready) {
        std::cerr << "Timed out waiting for Social SDK authorization/connection.\n";
        return 1;
    }
    std::cerr << (saw_activity
        ? "Player activity exists but assets.small_url is absent. Discovery may be disabled or Discord may have omitted the URL.\n"
        : "The Social SDK returned no activity for this application. Check the Application ID and Player playback.\n");
    return saw_activity ? 3 : 2;
}
