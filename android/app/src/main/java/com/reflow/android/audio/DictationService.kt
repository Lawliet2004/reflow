package com.reflow.android.audio

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.os.Build
import android.os.IBinder

class DictationService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val channelId = "reflow_dictation"
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val mgr = getSystemService(NotificationManager::class.java)
            mgr.createNotificationChannel(
                NotificationChannel(channelId, "Reflow dictation", NotificationManager.IMPORTANCE_LOW),
            )
            val notification = Notification.Builder(this, channelId)
                .setContentTitle("Reflow")
                .setContentText("Listening…")
                .setSmallIcon(android.R.drawable.ic_btn_speak_now)
                .build()
            if (Build.VERSION.SDK_INT >= 34) {
                startForeground(
                    7,
                    notification,
                    android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE,
                )
            } else {
                startForeground(7, notification)
            }
        }
        return START_STICKY
    }
}
