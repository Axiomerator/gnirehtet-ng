/*
 * Copyright (C) 2017 Genymobile
 * Modified for Gnirehtet NG by the Gnirehtet NG contributors.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package com.genymobile.gnirehtet;

import android.Manifest;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.res.ColorStateList;
import android.content.res.Configuration;
import android.graphics.Color;
import android.net.Uri;
import android.net.VpnService;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.PowerManager;
import android.util.Log;
import android.view.View;
import android.view.Window;
import android.widget.Button;
import android.widget.TextView;

import androidx.annotation.NonNull;
import androidx.appcompat.app.AppCompatActivity;
import androidx.core.graphics.Insets;
import androidx.core.view.ViewCompat;
import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsCompat;
import androidx.core.view.WindowInsetsControllerCompat;

import com.google.android.material.switchmaterial.SwitchMaterial;

import java.util.Locale;
import java.util.concurrent.TimeUnit;

/**
 * Material dashboard for Gnirehtet NG. The UI polls the VPN service state every second and
 * can be destroyed at any time without affecting the network service lifecycle.
 */
public class MainActivity extends AppCompatActivity {

    private static final String TAG = MainActivity.class.getSimpleName();

    private static final int VPN_REQUEST_CODE = 0;
    private static final int NOTIFICATION_PERMISSION_REQUEST_CODE = 1;
    private static final long REFRESH_PERIOD_MS = 1000;
    private static final long RESTART_DELAY_MS = 500;
    private static final long BYTE_UNIT = 1024;
    private static final long MILLIS_PER_SECOND = 1000;
    private static final int STATUS_DOT_NOT_RUNNING = 0xFF9AA7A1;
    private static final int STATUS_DOT_CONNECTING = 0xFFF2B531;
    private static final int STATUS_DOT_CONNECTED = 0xFF34C759;

    private final Handler refreshHandler = new Handler(Looper.getMainLooper());

    private VpnConfiguration requestedConfig;

    private TextView statusText;
    private View statusDot;
    private TextView pcText;
    private TextView downSpeed;
    private TextView downTotal;
    private TextView upSpeed;
    private TextView upTotal;
    private TextView uptimeText;
    private TextView dnsValue;
    private TextView routesValue;
    private SwitchMaterial autoSwitch;
    private Button startButton;
    private Button stopButton;
    private Button restartButton;
    private Button diagnosticsButton;
    private TextView batteryOptStatus;
    private Button batteryOptButton;

    private long lastDownBytes;
    private long lastUpBytes;
    private long lastSampleAtMs;

    private final Runnable refreshRunnable = new Runnable() {
        @Override
        public void run() {
            updateUi();
            refreshHandler.postDelayed(this, REFRESH_PERIOD_MS);
        }
    };

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_main);

        configureSystemBars();
        applySystemBarInsets();

        statusText = findViewById(R.id.status_text);
        statusDot = findViewById(R.id.status_dot);
        pcText = findViewById(R.id.pc_text);
        downSpeed = findViewById(R.id.down_speed);
        downTotal = findViewById(R.id.down_total);
        upSpeed = findViewById(R.id.up_speed);
        upTotal = findViewById(R.id.up_total);
        uptimeText = findViewById(R.id.uptime_text);
        dnsValue = findViewById(R.id.dns_value);
        routesValue = findViewById(R.id.routes_value);
        autoSwitch = findViewById(R.id.auto_switch);
        startButton = findViewById(R.id.start_button);
        stopButton = findViewById(R.id.stop_button);
        restartButton = findViewById(R.id.restart_button);
        diagnosticsButton = findViewById(R.id.diagnostics_button);
        batteryOptStatus = findViewById(R.id.battery_opt_status);
        batteryOptButton = findViewById(R.id.battery_opt_button);

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            batteryOptButton.setOnClickListener(view -> {
                Intent intent = new Intent(android.provider.Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS,
                        Uri.parse("package:" + getPackageName()));
                try {
                    startActivity(intent);
                } catch (RuntimeException error) {
                    Log.w(TAG, "Could not open battery optimization settings: " + error);
                }
            });
        } else {
            findViewById(R.id.background_reliability_card).setVisibility(View.GONE);
        }

        downSpeed.setText(formatBytes(0, true));
        upSpeed.setText(formatBytes(0, true));
        downTotal.setText(formatBytes(0));
        upTotal.setText(formatBytes(0));

        autoSwitch.setChecked(Settings.isAutoReconnectEnabled(this));
        autoSwitch.setOnCheckedChangeListener((buttonView, isChecked) ->
                Settings.setAutoReconnectEnabled(MainActivity.this, isChecked));

        startButton.setOnClickListener(view -> requestStart());
        stopButton.setOnClickListener(view -> GnirehtetService.stop(MainActivity.this));
        restartButton.setOnClickListener(view -> {
            GnirehtetService.stop(MainActivity.this);
            refreshHandler.postDelayed(this::requestStart, RESTART_DELAY_MS);
        });
        diagnosticsButton.setOnClickListener(view -> exportDiagnostics());

        updateUi();
        handleIntent(getIntent());
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        handleIntent(intent);
    }

    /**
     * Process PC-driven START/STOP actions. When the app is not in the foreground, the system
     * delivers these intents to this activity (singleTop) instead of launching a new instance.
     */
    private void handleIntent(Intent intent) {
        String action = intent.getAction();
        if (GnirehtetActivity.ACTION_GNIREHTET_START.equals(action)) {
            requestedConfig = configFromIntent(intent);
            requestStart();
        } else if (GnirehtetActivity.ACTION_GNIREHTET_STOP.equals(action)) {
            GnirehtetService.stop(this);
        }
    }

    private static VpnConfiguration configFromIntent(Intent intent) {
        String[] dnsServers = intent.getStringArrayExtra(GnirehtetActivity.EXTRA_DNS_SERVERS);
        if (dnsServers == null) {
            dnsServers = new String[0];
        }
        String[] routes = intent.getStringArrayExtra(GnirehtetActivity.EXTRA_ROUTES);
        if (routes == null) {
            routes = new String[0];
        }
        return new VpnConfiguration(Net.toInetAddresses(dnsServers), Net.toCIDRs(routes));
    }

    /**
     * Draw edge-to-edge and let the system choose the correct system-bar icon contrast for
     * the active theme: dark icons on the light background, light icons in dark mode.
     */
    private void configureSystemBars() {
        Window window = getWindow();
        WindowCompat.setDecorFitsSystemWindows(window, false);
        window.setStatusBarColor(Color.TRANSPARENT);
        window.setNavigationBarColor(Color.TRANSPARENT);
        int nightMode = getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK;
        boolean lightBackground = nightMode != Configuration.UI_MODE_NIGHT_YES;
        WindowInsetsControllerCompat controller =
                WindowCompat.getInsetsController(window, window.getDecorView());
        controller.setAppearanceLightStatusBars(lightBackground);
        controller.setAppearanceLightNavigationBars(lightBackground);
    }

    private void applySystemBarInsets() {
        View root = findViewById(android.R.id.content);
        ViewCompat.setOnApplyWindowInsetsListener(root, (view, windowInsets) -> {
            Insets bars = windowInsets.getInsets(WindowInsetsCompat.Type.systemBars());
            view.setPadding(bars.left, bars.top, bars.right, bars.bottom);
            return WindowInsetsCompat.CONSUMED;
        });
    }

    @Override
    protected void onResume() {
        super.onResume();
        refreshHandler.removeCallbacks(refreshRunnable);
        refreshHandler.post(refreshRunnable);
    }

    @Override
    protected void onPause() {
        super.onPause();
        refreshHandler.removeCallbacks(refreshRunnable);
    }

    private void updateUi() {
        boolean running = GnirehtetService.isRunning();
        boolean connected = GnirehtetService.isTunnelConnected();
        int dotColor = STATUS_DOT_NOT_RUNNING;
        if (!running) {
            statusText.setText(R.string.status_not_running);
        } else if (connected) {
            statusText.setText(R.string.status_connected);
            dotColor = STATUS_DOT_CONNECTED;
        } else {
            statusText.setText(R.string.status_connecting);
            dotColor = STATUS_DOT_CONNECTING;
        }
        statusDot.setBackgroundTintList(ColorStateList.valueOf(dotColor));

        String pcName = SessionInfo.getPcName();
        if (running && !pcName.isEmpty()) {
            pcText.setText(getString(R.string.pc_connected_format, pcName, SessionInfo.getRelayPort()));
            pcText.setVisibility(View.VISIBLE);
        } else {
            pcText.setVisibility(View.GONE);
        }

        String dns = SessionInfo.getDnsServers();
        dnsValue.setText(dns.isEmpty() ? getString(R.string.value_not_set) : dns);
        String routes = SessionInfo.getRoutes();
        routesValue.setText(routes.isEmpty() ? getString(R.string.value_not_set) : routes);

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            PowerManager powerManager = (PowerManager) getSystemService(POWER_SERVICE);
            boolean batteryExempt = powerManager != null && powerManager.isIgnoringBatteryOptimizations(getPackageName());
            batteryOptStatus.setText(batteryExempt
                    ? R.string.battery_opt_exempt : R.string.battery_opt_not_exempt);
            batteryOptButton.setVisibility(batteryExempt ? View.GONE : View.VISIBLE);
        }

        long now = System.currentTimeMillis();
        long down = GnirehtetService.getDownBytes();
        long up = GnirehtetService.getUpBytes();
        long elapsed = Math.max(1, now - lastSampleAtMs);
        if (lastSampleAtMs != 0) {
            long downRate = (down - lastDownBytes) * MILLIS_PER_SECOND / elapsed;
            long upRate = (up - lastUpBytes) * MILLIS_PER_SECOND / elapsed;
            downSpeed.setText(formatBytes(downRate, true));
            upSpeed.setText(formatBytes(upRate, true));
        }
        downTotal.setText(formatBytes(down));
        upTotal.setText(formatBytes(up));
        lastDownBytes = down;
        lastUpBytes = up;
        lastSampleAtMs = now;

        long startedAt = GnirehtetService.getStartedAtMs();
        if (running && startedAt != 0) {
            long uptimeSeconds = TimeUnit.MILLISECONDS.toSeconds(now - startedAt);
            uptimeText.setText(getString(R.string.uptime_format, formatUptime(uptimeSeconds)));
            uptimeText.setVisibility(View.VISIBLE);
        } else {
            uptimeText.setVisibility(View.GONE);
        }

        startButton.setEnabled(!running);
        stopButton.setEnabled(running);
        restartButton.setEnabled(running);
    }

    private void requestStart() {
        if (GnirehtetService.isRunning()) {
            return;
        }
        prepareAndStart();
    }

    private void prepareAndStart() {
        VpnConfiguration config = requestedConfig != null ? requestedConfig : new VpnConfiguration();
        Intent vpnIntent = VpnService.prepare(this);
        if (vpnIntent == null) {
            GnirehtetService.start(this, config);
            requestNotificationPermissionIfNeeded();
            return;
        }
        requestedConfig = config;
        startActivityForResult(vpnIntent, VPN_REQUEST_CODE);
    }

    /**
     * Request the notification permission at most once (persisted). Never blocks the VPN start.
     */
    private void requestNotificationPermissionIfNeeded() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            return;
        }
        if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                == PackageManager.PERMISSION_GRANTED) {
            return;
        }
        if (Settings.isNotificationPermissionRequested(this)) {
            return;
        }
        Settings.setNotificationPermissionRequested(this, true);
        requestPermissions(new String[] {Manifest.permission.POST_NOTIFICATIONS},
                NOTIFICATION_PERMISSION_REQUEST_CODE);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == VPN_REQUEST_CODE && resultCode == RESULT_OK) {
            VpnConfiguration config = requestedConfig != null ? requestedConfig : new VpnConfiguration();
            GnirehtetService.start(this, config);
            requestNotificationPermissionIfNeeded();
        }
        requestedConfig = null;
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, @NonNull String[] permissions,
            @NonNull int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        if (requestCode == NOTIFICATION_PERMISSION_REQUEST_CODE) {
            Log.d(TAG, "Notification permission result delivered");
        }
    }

    private void exportDiagnostics() {
        StringBuilder summary = new StringBuilder();
        summary.append("Gnirehtet NG diagnostics\n");
        summary.append("running=").append(GnirehtetService.isRunning()).append('\n');
        summary.append("tunnel_connected=").append(GnirehtetService.isTunnelConnected()).append('\n');
        summary.append("pc_name=").append(SessionInfo.getPcName()).append('\n');
        summary.append("relay_port=").append(SessionInfo.getRelayPort()).append('\n');
        summary.append("dns_servers=").append(SessionInfo.getDnsServers()).append('\n');
        summary.append("routes=").append(SessionInfo.getRoutes()).append('\n');
        summary.append("down_bytes=").append(GnirehtetService.getDownBytes()).append('\n');
        summary.append("up_bytes=").append(GnirehtetService.getUpBytes()).append('\n');
        summary.append("started_at_ms=").append(GnirehtetService.getStartedAtMs()).append('\n');
        summary.append("auto_reconnect=").append(Settings.isAutoReconnectEnabled(this)).append('\n');
        summary.append("android=").append(Build.VERSION.RELEASE)
                .append(" (API ").append(Build.VERSION.SDK_INT).append(")\n");

        Intent share = new Intent(Intent.ACTION_SEND);
        share.setType("text/plain");
        share.putExtra(Intent.EXTRA_SUBJECT, getString(R.string.diagnostics_subject));
        share.putExtra(Intent.EXTRA_TEXT, summary.toString());
        startActivity(Intent.createChooser(share, getString(R.string.action_diagnostics)));
    }

    @SuppressWarnings("checkstyle:MagicNumber")
    private static String formatBytes(long bytes) {
        return formatBytes(bytes, false);
    }

    @SuppressWarnings("checkstyle:MagicNumber")
    private static String formatBytes(long bytes, boolean perSecond) {
        if (bytes < BYTE_UNIT) {
            return bytes + " B" + (perSecond ? "/s" : "");
        }
        double value = bytes;
        String[] units = {"KiB", "MiB", "GiB", "TiB"};
        int unit = -1;
        while (value >= BYTE_UNIT && unit + 1 < units.length) {
            value /= BYTE_UNIT;
            unit++;
        }
        return String.format(Locale.US, "%.1f %s%s", value, units[unit], perSecond ? "/s" : "");
    }

    @SuppressWarnings("checkstyle:MagicNumber")
    private static String formatUptime(long seconds) {
        if (seconds < 60) {
            return seconds + "s";
        }
        long minutes = seconds / 60;
        if (minutes < 60) {
            return minutes + "m " + (seconds % 60) + "s";
        }
        long hours = minutes / 60;
        return hours + "h " + (minutes % 60) + "m";
    }
}
