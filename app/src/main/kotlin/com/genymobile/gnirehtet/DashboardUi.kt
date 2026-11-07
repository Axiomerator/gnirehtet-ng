/*
 * Copyright 2026 Gnirehtet NG contributors
 * SPDX-License-Identifier: Apache-2.0
 */
package com.genymobile.gnirehtet

import android.content.Context
import android.os.PowerManager
import android.os.SystemClock
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.platform.ViewCompositionStrategy
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.view.WindowCompat
import ui.feice.mochi.basic.MochiButton
import ui.feice.mochi.basic.MochiButtonStyle
import ui.feice.mochi.basic.MochiSwitch
import ui.feice.mochi.basic.Text
import ui.feice.mochi.theme.ColorSchemeMode
import ui.feice.mochi.theme.MochiTheme
import ui.feice.mochi.theme.ThemeController
import java.util.Locale

internal class DashboardUi(private val activity: MainActivity) {
    private val preferences = activity.getSharedPreferences("gnirehtet_ng_settings", Context.MODE_PRIVATE)
    private var appearance by mutableIntStateOf(preferences.getInt("appearance", 0).coerceIn(0, 2))
    private var data by mutableStateOf(Snapshot())
    private var sampleTime = 0L
    private var sampleSession = 0L
    private var sampleDown = 0L
    private var sampleUp = 0L

    init {
        activity.setContentView(ComposeView(activity).apply {
            setViewCompositionStrategy(ViewCompositionStrategy.DisposeOnViewTreeLifecycleDestroyed)
            setContent { Screen() }
        })
        refresh()
    }

    fun refresh() {
        val now = SystemClock.elapsedRealtime()
        val started = GnirehtetService.getStartedAtMs()
        val down = GnirehtetService.getDownBytes()
        val up = GnirehtetService.getUpBytes()
        val sameSession = started != 0L && started == sampleSession && now > sampleTime
        val power = activity.getSystemService(PowerManager::class.java)
        data = Snapshot(
            running = GnirehtetService.isRunning(),
            connected = GnirehtetService.isTunnelConnected(),
            pc = SessionInfo.getPcName(),
            port = SessionInfo.getRelayPort(),
            dns = SessionInfo.getDnsServers(),
            routes = SessionInfo.getRoutes(),
            down = down,
            up = up,
            downRate = if (sameSession) (down - sampleDown).coerceAtLeast(0L) * 1000 / (now - sampleTime) else 0L,
            upRate = if (sameSession) (up - sampleUp).coerceAtLeast(0L) * 1000 / (now - sampleTime) else 0L,
            elapsed = if (started == 0L) 0L else (System.currentTimeMillis() - started).coerceAtLeast(0L) / 1000,
            autoReconnect = Settings.isAutoReconnectEnabled(activity),
            batteryExempt = power?.isIgnoringBatteryOptimizations(activity.packageName) == true,
        )
        sampleTime = now
        sampleSession = started
        sampleDown = down
        sampleUp = up
    }

    @Composable
    private fun Screen() {
        val dark = when (appearance) {
            1 -> false
            2 -> true
            else -> isSystemInDarkTheme()
        }
        val colors = if (dark) Dark else Light
        val controller = remember(dark) {
            ThemeController(if (dark) ColorSchemeMode.Dark else ColorSchemeMode.Light, keyColor = Color(0xFF278463))
        }
        SideEffect {
            WindowCompat.getInsetsController(activity.window, activity.window.decorView).apply {
                isAppearanceLightStatusBars = !dark
                isAppearanceLightNavigationBars = !dark
            }
        }
        MochiTheme(controller = controller) {
            Box(
                Modifier.fillMaxSize().background(colors.background).windowInsetsPadding(WindowInsets.safeDrawing),
                contentAlignment = Alignment.TopCenter,
            ) {
                Column(
                    Modifier.widthIn(max = 620.dp).fillMaxWidth().verticalScroll(rememberScrollState()).padding(20.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                        Label("GNIREHTET / NG", colors.accent, 11, FontWeight.Bold, mono = true)
                        Label(stringResource(R.string.dashboard_title), colors.text, 31, FontWeight.Bold)
                        Label(stringResource(R.string.dashboard_subtitle), colors.muted, 13)
                    }

                    Panel(colors, featured = true) {
                        val status = stringResource(when {
                            !data.running -> R.string.status_not_running
                            data.connected -> R.string.status_connected
                            else -> R.string.status_connecting
                        })
                        val statusColor = when {
                            !data.running -> colors.muted
                            data.connected -> colors.accent
                            else -> colors.warning
                        }
                        Row(
                            Modifier.semantics { stateDescription = status },
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Box(Modifier.size(8.dp).background(statusColor, CircleShape))
                            Label(status, colors.text, 18, FontWeight.SemiBold)
                        }
                        if (data.running) {
                            val peer = if (data.pc.isBlank()) stringResource(R.string.dashboard_subtitle)
                                else stringResource(R.string.pc_connected_format, data.pc, data.port)
                            Label(peer, colors.muted, 12)
                            Label(stringResource(R.string.uptime_format, uptime(data.elapsed)), colors.muted, 12, mono = true)
                        }
                        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            MochiButton(onClick = activity::requestStart, enabled = !data.running, modifier = Modifier.weight(1f)) {
                                Text(stringResource(R.string.action_start))
                            }
                            MochiButton(
                                onClick = activity::stopVpn,
                                enabled = data.running,
                                style = MochiButtonStyle.Tonal,
                                modifier = Modifier.weight(1f),
                            ) { Text(stringResource(R.string.action_stop)) }
                            MochiButton(
                                onClick = activity::restartVpn,
                                enabled = data.running,
                                style = MochiButtonStyle.Tonal,
                                modifier = Modifier.weight(1f),
                            ) { Text(stringResource(R.string.action_restart)) }
                        }
                    }

                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        TrafficCell(Modifier.weight(1f), colors, stringResource(R.string.traffic_down), data.downRate, data.down)
                        TrafficCell(Modifier.weight(1f), colors, stringResource(R.string.traffic_up), data.upRate, data.up)
                    }

                    Panel(colors) {
                        Label(stringResource(R.string.vpn_configuration_title), colors.text, 15, FontWeight.SemiBold)
                        Detail(colors, stringResource(R.string.dns_label), data.dns)
                        Detail(colors, stringResource(R.string.routes_label), data.routes)
                    }

                    Panel(colors) {
                        Label(stringResource(R.string.background_reliability_title), colors.text, 15, FontWeight.SemiBold)
                        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp),
                            verticalAlignment = Alignment.CenterVertically) {
                            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                Label(stringResource(R.string.auto_reconnect), colors.text, 14)
                                Label(stringResource(R.string.auto_reconnect_hint), colors.muted, 12)
                            }
                            MochiSwitch(data.autoReconnect, { enabled ->
                                Settings.setAutoReconnectEnabled(activity, enabled)
                                refresh()
                            })
                        }
                        Label(
                            stringResource(if (data.batteryExempt) R.string.battery_opt_exempt else R.string.battery_opt_not_exempt),
                            colors.muted, 12,
                        )
                        if (!data.batteryExempt) {
                            MochiButton(onClick = activity::openBatteryOptimization, style = MochiButtonStyle.Tonal) {
                                Text(stringResource(R.string.action_authorize_battery_opt))
                            }
                        }
                    }

                    Panel(colors) {
                        Label(stringResource(R.string.appearance), colors.text, 15, FontWeight.SemiBold)
                        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            listOf(R.string.appearance_system, R.string.appearance_light, R.string.appearance_dark)
                                .forEachIndexed { index, title ->
                                    MochiButton(
                                        onClick = {
                                            appearance = index
                                            preferences.edit().putInt("appearance", index).apply()
                                        },
                                        modifier = Modifier.weight(1f),
                                        style = if (appearance == index) MochiButtonStyle.Filled else MochiButtonStyle.Tonal,
                                    ) { Text(stringResource(title)) }
                                }
                        }
                        MochiButton(onClick = activity::exportDiagnostics, style = MochiButtonStyle.Tonal) {
                            Text(stringResource(R.string.action_diagnostics))
                        }
                    }
                }
            }
        }
    }
}

private data class Snapshot(
    val running: Boolean = false,
    val connected: Boolean = false,
    val pc: String = "",
    val port: Int = 0,
    val dns: String = "",
    val routes: String = "",
    val down: Long = 0,
    val up: Long = 0,
    val downRate: Long = 0,
    val upRate: Long = 0,
    val elapsed: Long = 0,
    val autoReconnect: Boolean = true,
    val batteryExempt: Boolean = false,
)

private data class Palette(
    val background: Color, val surface: Color, val feature: Color, val border: Color,
    val text: Color, val muted: Color, val accent: Color, val warning: Color,
)

private val Light = Palette(
    Color(0xFFF3F6F4), Color(0xFFFFFFFF), Color(0xFFE7F1EC), Color(0xFFDBE4DE),
    Color(0xFF15251E), Color(0xFF53675C), Color(0xFF17644A), Color(0xFF895019),
)
private val Dark = Palette(
    Color(0xFF101714), Color(0xFF18221D), Color(0xFF1B3026), Color(0xFF2C3B33),
    Color(0xFFEAF3ED), Color(0xFFA7BBB0), Color(0xFF8DDEB3), Color(0xFFF2BC71),
)

@Composable
private fun Panel(
    colors: Palette, modifier: Modifier = Modifier, featured: Boolean = false, content: @Composable ColumnScope.() -> Unit,
) {
    val shape = RoundedCornerShape(if (featured) 26.dp else 20.dp)
    Column(
        modifier.fillMaxWidth().background(if (featured) colors.feature else colors.surface, shape)
            .border(1.dp, colors.border, shape).padding(15.dp),
        verticalArrangement = Arrangement.spacedBy(9.dp),
        content = content,
    )
}

@Composable
private fun TrafficCell(modifier: Modifier, colors: Palette, title: String, rate: Long, total: Long) {
    Panel(colors, modifier) {
        Label(title, colors.muted, 12)
        Label(bytes(rate) + "/s", colors.text, 19, FontWeight.SemiBold, mono = true)
        Label(bytes(total), colors.muted, 12, mono = true)
    }
}

@Composable
private fun Detail(colors: Palette, title: String, value: String) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        Box(Modifier.weight(1f)) { Label(title, colors.muted, 13) }
        Box(Modifier.weight(1.3f)) {
            Label(value.ifBlank { stringResource(R.string.value_not_set) }, colors.text, 13, mono = true)
        }
    }
}

@Composable
private fun Label(value: String, color: Color, size: Int, weight: FontWeight = FontWeight.Normal, mono: Boolean = false) {
    BasicText(
        value, style = TextStyle(
            color = color, fontSize = size.sp, lineHeight = (size + 5).sp, fontWeight = weight,
            fontFamily = if (mono) FontFamily.Monospace else FontFamily.SansSerif,
        ),
    )
}

private fun bytes(value: Long): String {
    val units = listOf("B", "KiB", "MiB", "GiB", "TiB")
    var amount = value.toDouble()
    var unit = 0
    while (amount >= 1024 && unit < units.lastIndex) {
        amount /= 1024
        unit++
    }
    return if (unit == 0) "$value B" else String.format(Locale.getDefault(), "%.1f %s", amount, units[unit])
}

private fun uptime(seconds: Long): String = String.format(
    Locale.getDefault(), "%02d:%02d:%02d", seconds / 3600, seconds / 60 % 60, seconds % 60,
)
