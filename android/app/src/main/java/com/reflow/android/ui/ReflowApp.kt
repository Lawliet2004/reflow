package com.reflow.android.ui

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import com.reflow.android.audio.DictationService
import com.reflow.android.audio.PcmRecorder
import com.reflow.android.data.HistoryItem
import com.reflow.android.data.Prefs
import com.reflow.android.data.ServerConnection
import com.reflow.android.net.DictationSocket
import com.reflow.android.net.ReflowClient
import com.reflow.android.net.parsePairUri
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private val Zinc = darkColorScheme(
    primary = Color(0xFF6366F1),
    background = Color(0xFF09090B),
    surface = Color(0xFF18181B),
    onBackground = Color(0xFFFAFAFA),
    onSurface = Color(0xFFFAFAFA),
)

@Composable
fun ReflowApp(initialPairUri: String?) {
    val context = LocalContext.current
    val prefs = remember { Prefs(context) }
    val client = remember { ReflowClient() }
    var connection by remember { mutableStateOf(prefs.connection) }
    var tab by remember { mutableStateOf(if (connection == null) "pair" else "dictate") }

    MaterialTheme(colorScheme = Zinc) {
        Column(
            Modifier
                .fillMaxSize()
                .background(Zinc.background)
                .padding(20.dp),
        ) {
            Text("Reflow", color = Color.White, fontSize = 22.sp)
            Text(
                connection?.serverName ?: "Not paired",
                color = Color(0xFFA1A1AA),
                fontSize = 12.sp,
            )
            Spacer(Modifier.height(16.dp))
            if (connection != null) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextButton(onClick = { tab = "dictate" }) { Text("Dictate") }
                    TextButton(onClick = { tab = "history" }) { Text("History") }
                    TextButton(onClick = { tab = "settings" }) { Text("Settings") }
                }
            }
            when {
                connection == null || tab == "pair" -> PairScreen(
                    initialPairUri = initialPairUri,
                    client = client,
                    onPaired = {
                        prefs.connection = it
                        connection = it
                        tab = "dictate"
                    },
                )
                tab == "history" -> HistoryScreen(client, connection!!)
                tab == "settings" -> SettingsScreen(
                    prefs = prefs,
                    onForget = {
                        prefs.connection = null
                        connection = null
                        tab = "pair"
                    },
                )
                else -> DictateScreen(prefs, client, connection!!)
            }
        }
    }
}

@Composable
private fun PairScreen(
    initialPairUri: String?,
    client: ReflowClient,
    onPaired: (ServerConnection) -> Unit,
) {
    val scope = rememberCoroutineScope()
    var host by remember { mutableStateOf("192.168.1.1") }
    var port by remember { mutableStateOf("7840") }
    var code by remember { mutableStateOf("") }
    var pairUri by remember { mutableStateOf(initialPairUri ?: "") }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }

    fun applyPairUri(value: String) {
        pairUri = value
        parsePairUri(value)?.let { (h, p, c) ->
            host = h
            port = p.toString()
            code = c
        }
    }

    LaunchedEffect(initialPairUri) {
        initialPairUri?.let { applyPairUri(it) }
    }

    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text("Pair with your Windows or Linux desktop.", color = Color(0xFFA1A1AA), fontSize = 13.sp)
        Text(
            "Paste the pair link from desktop Settings → Phone, or enter IP + code.",
            color = Color(0xFFA1A1AA),
            fontSize = 13.sp,
        )
        OutlinedTextField(
            pairUri,
            { applyPairUri(it) },
            label = { Text("Pair link") },
            placeholder = { Text("reflow://pair?host=...") },
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedTextField(
            code,
            { code = it },
            label = { Text("6-digit code") },
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedTextField(host, { host = it }, label = { Text("Desktop IP") }, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(port, { port = it }, label = { Text("Port") }, modifier = Modifier.fillMaxWidth())
        if (error != null) Text(error!!, color = Color(0xFFF87171), fontSize = 12.sp)
        Button(
            enabled = !busy,
            onClick = {
                busy = true
                error = null
                scope.launch {
                    try {
                        val conn = withContext(Dispatchers.IO) {
                            client.pair(host.trim(), port.toIntOrNull() ?: 7840, code.trim(), "Android")
                        }
                        onPaired(conn)
                    } catch (e: Exception) {
                        error = e.message
                    } finally {
                        busy = false
                    }
                }
            },
        ) { Text(if (busy) "Pairing…" else "Pair") }
    }
}

@Composable
private fun DictateScreen(prefs: Prefs, client: ReflowClient, conn: ServerConnection) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var listening by remember { mutableStateOf(false) }
    var transcript by remember { mutableStateOf("Hold to talk") }
    var error by remember { mutableStateOf<String?>(null) }
    var socket by remember { mutableStateOf<DictationSocket?>(null) }
    var recorder by remember { mutableStateOf<PcmRecorder?>(null) }

    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (!granted) error = "Microphone permission required"
    }

    fun stopSession() {
        recorder?.stop()
        recorder = null
        socket?.stop()
        listening = false
        context.stopService(Intent(context, DictationService::class.java))
    }

    fun startSession() {
        val hasMic = ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED
        if (!hasMic) {
            permission.launch(Manifest.permission.RECORD_AUDIO)
            return
        }
        error = null
        transcript = "Listening…"
        context.startForegroundService(Intent(context, DictationService::class.java))
        val sock = client.openStream(
            conn = conn,
            language = prefs.language,
            inject = prefs.injectOnDesktop,
            onPartial = { transcript = it.fullText.ifBlank { "Listening…" } },
            onFinal = {
                transcript = it.text.ifBlank { transcript }
                listening = false
                recorder?.stop()
                recorder = null
                context.stopService(Intent(context, DictationService::class.java))
            },
            onError = { error = it; stopSession() },
            onReady = { listening = true },
        )
        socket = sock
        val rec = PcmRecorder { bytes -> sock.sendPcm(bytes) }
        recorder = rec
        rec.start()
        listening = true
    }

    Column(Modifier.fillMaxSize(), verticalArrangement = Arrangement.SpaceBetween) {
        Column {
            Text(transcript, color = Color.White, fontSize = 18.sp)
            if (error != null) Text(error!!, color = Color(0xFFF87171), fontSize = 12.sp)
        }
        Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.fillMaxWidth()) {
            val hold = Modifier
                .fillMaxWidth()
                .height(72.dp)
                .background(if (listening) Color(0xFFDC2626) else Color(0xFF4F46E5), RoundedCornerShape(20.dp))
                .pointerInput(listening) {
                    detectTapGestures(
                        onPress = {
                            startSession()
                            tryAwaitRelease()
                            stopSession()
                        },
                    )
                }
            Box(hold, contentAlignment = Alignment.Center) {
                Text(if (listening) "Release to finish" else "Hold to talk", color = Color.White)
            }
            Spacer(Modifier.height(12.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                TextButton(onClick = {
                    val cm = context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
                    cm.setPrimaryClip(android.content.ClipData.newPlainText("reflow", transcript))
                }) { Text("Copy") }
                TextButton(onClick = {
                    scope.launch(Dispatchers.IO) {
                        runCatching { client.inject(conn, transcript) }
                    }
                }) { Text("Paste on PC") }
            }
        }
    }
}

@Composable
private fun HistoryScreen(client: ReflowClient, conn: ServerConnection) {
    var items by remember { mutableStateOf<List<HistoryItem>>(emptyList()) }
    var error by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(conn.token) {
        try {
            items = withContext(Dispatchers.IO) { client.history(conn) }
        } catch (e: Exception) {
            error = e.message
        }
    }
    if (error != null) Text(error!!, color = Color(0xFFF87171))
    LazyColumn(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        items(items, key = { it.id }) { item ->
            Column(
                Modifier
                    .fillMaxWidth()
                    .background(Color(0xFF18181B), RoundedCornerShape(12.dp))
                    .padding(12.dp),
            ) {
                Text(item.text, color = Color.White, fontSize = 14.sp)
                Text("${item.language} · ${item.createdAt}", color = Color(0xFF71717A), fontSize = 11.sp)
            }
        }
    }
}

@Composable
private fun SettingsScreen(prefs: Prefs, onForget: () -> Unit) {
    var inject by remember { mutableStateOf(prefs.injectOnDesktop) }
    var language by remember { mutableStateOf(prefs.language) }
    Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text("Paste on computer when I stop", color = Color.White)
            Switch(inject, {
                inject = it
                prefs.injectOnDesktop = it
            })
        }
        OutlinedTextField(
            language,
            {
                language = it
                prefs.language = it
            },
            label = { Text("Language (auto/en/hi/bn)") },
            modifier = Modifier.fillMaxWidth(),
        )
        Text("ASR stays on the desktop. This phone is only a microphone.", color = Color(0xFFA1A1AA), fontSize = 12.sp)
        Button(onClick = onForget) { Text("Forget this computer") }
    }
}
