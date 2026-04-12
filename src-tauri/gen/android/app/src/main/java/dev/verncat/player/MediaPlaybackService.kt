package dev.verncat.player

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.drawable.Icon
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Build
import android.os.IBinder

class MediaPlaybackService : Service() {

    private lateinit var mediaSession: MediaSession
    private lateinit var nm: NotificationManager

    private var currentTitle = ""
    private var currentArtist = ""
    private var currentIsPlaying = false
    private var currentPositionMs = 0L
    private var currentDurationMs = 0L
    private var started = false

    companion object {
        const val ACTION_UPDATE = "dev.verncat.player.UPDATE"
        const val ACTION_PREV   = "dev.verncat.player.PREV"
        const val ACTION_NEXT   = "dev.verncat.player.NEXT"
        const val ACTION_PLAY   = "dev.verncat.player.PLAY"
        const val ACTION_PAUSE  = "dev.verncat.player.PAUSE"
        const val EXTRA_TITLE      = "title"
        const val EXTRA_ARTIST     = "artist"
        const val EXTRA_IS_PLAYING = "is_playing"
        const val EXTRA_POSITION   = "position"
        const val EXTRA_DURATION   = "duration"
        const val CHANNEL_ID = "player_playback"
        const val NOTIF_ID   = 1
    }

    override fun onCreate() {
        super.onCreate()
        nm = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_ID, "Playback", NotificationManager.IMPORTANCE_LOW).apply {
                setShowBadge(false)
            }
        )
        mediaSession = MediaSession(this, "PlayerSession").apply {
            setCallback(object : MediaSession.Callback() {
                override fun onPlay()                = forward("play")
                override fun onPause()               = forward("pause")
                override fun onSkipToNext()          = forward("next")
                override fun onSkipToPrevious()      = forward("prev")
                override fun onSeekTo(pos: Long)     = forward("seek:${pos / 1000}")
            })
            isActive = true
        }
    }

    private fun forward(action: String) {
        MainActivity.instance?.get()?.sendToJs(action)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_UPDATE -> {
                currentTitle      = intent.getStringExtra(EXTRA_TITLE)     ?: currentTitle
                currentArtist     = intent.getStringExtra(EXTRA_ARTIST)    ?: currentArtist
                currentIsPlaying  = intent.getBooleanExtra(EXTRA_IS_PLAYING, currentIsPlaying)
                currentPositionMs = intent.getLongExtra(EXTRA_POSITION, currentPositionMs)
                currentDurationMs = intent.getLongExtra(EXTRA_DURATION, currentDurationMs)
            }
            ACTION_PREV  -> forward("prev")
            ACTION_NEXT  -> forward("next")
            ACTION_PLAY  -> forward("play")
            ACTION_PAUSE -> { currentIsPlaying = false; forward("pause") }
        }
        updateSession()
        val notif = buildNotification()
        if (!started) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                startForeground(NOTIF_ID, notif, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
            } else {
                startForeground(NOTIF_ID, notif)
            }
            started = true
        } else {
            nm.notify(NOTIF_ID, notif)
        }
        return START_STICKY
    }

    private fun updateSession() {
        mediaSession.setMetadata(
            MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, currentTitle)
                .putString(MediaMetadata.METADATA_KEY_ARTIST, currentArtist)
                .putLong(MediaMetadata.METADATA_KEY_DURATION, currentDurationMs)
                .build()
        )
        val state = if (currentIsPlaying) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED
        mediaSession.setPlaybackState(
            PlaybackState.Builder()
                .setActions(
                    PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or
                    PlaybackState.ACTION_SKIP_TO_NEXT or PlaybackState.ACTION_SKIP_TO_PREVIOUS or
                    PlaybackState.ACTION_SEEK_TO
                )
                .setState(state, currentPositionMs, 1f)
                .build()
        )
    }

    private fun svcPendingIntent(action: String, code: Int): PendingIntent =
        PendingIntent.getService(
            this, code,
            Intent(this, MediaPlaybackService::class.java).apply { this.action = action },
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

    private fun buildNotification(): Notification {
        val launchPI = PendingIntent.getActivity(
            this, 0,
            packageManager.getLaunchIntentForPackage(packageName)?.apply {
                flags = Intent.FLAG_ACTIVITY_SINGLE_TOP
            },
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val playPauseIcon  = if (currentIsPlaying) android.R.drawable.ic_media_pause
                             else android.R.drawable.ic_media_play
        val playPauseLabel = if (currentIsPlaying) "Pause" else "Play"
        val playPauseAct   = if (currentIsPlaying) ACTION_PAUSE else ACTION_PLAY

        return Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setContentTitle(currentTitle.ifEmpty { "Player" })
            .setContentText(currentArtist)
            .setContentIntent(launchPI)
            .setOngoing(currentIsPlaying)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .setStyle(
                Notification.MediaStyle()
                    .setMediaSession(mediaSession.sessionToken)
                    .setShowActionsInCompactView(0, 1, 2)
            )
            .addAction(
                Notification.Action.Builder(
                    Icon.createWithResource(this, android.R.drawable.ic_media_previous),
                    "Previous", svcPendingIntent(ACTION_PREV, 1)
                ).build()
            )
            .addAction(
                Notification.Action.Builder(
                    Icon.createWithResource(this, playPauseIcon),
                    playPauseLabel, svcPendingIntent(playPauseAct, 2)
                ).build()
            )
            .addAction(
                Notification.Action.Builder(
                    Icon.createWithResource(this, android.R.drawable.ic_media_next),
                    "Next", svcPendingIntent(ACTION_NEXT, 3)
                ).build()
            )
            .build()
    }

    override fun onDestroy() {
        super.onDestroy()
        mediaSession.release()
    }

    override fun onBind(intent: Intent?): IBinder? = null
}
