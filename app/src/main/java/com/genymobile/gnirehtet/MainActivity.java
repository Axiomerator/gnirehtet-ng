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
import android.graphics.Color;
import android.net.Uri;
import android.net.VpnService;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.Window;

import androidx.annotation.NonNull;
import androidx.appcompat.app.AppCompatActivity;
import androidx.core.view.WindowCompat;



/**
 * Compose dashboard for Gnirehtet NG. The UI polls the VPN service state every second and
 * can be destroyed at any time without affecting the network service lifecycle.
 */
public class MainActivity extends AppCompatActivity {

    private static final String TAG = MainActivity.class.getSimpleName();

    private static final int VPN_REQUEST_CODE = 0;
    private static final int NOTIFICATION_PERMISSION_REQUEST_CODE = 1;
    private static final long REFRESH_PERIOD_MS = 1000;
    private static final long RESTART_DELAY_MS = 500;
    private final Handler refreshHandler = new Handler(Looper.getMainLooper());
    private final Runnable restartRunnable = () -> prepareAndStart(false);
    private VpnConfiguration requestedConfig;
    private boolean vpnConsentPending;
    private boolean startRequestedByPhone;
    private DashboardUi dashboard;

    private final Runnable refreshRunnable = new Runnable() {
        @Override
        public void run() {
            dashboard.refresh();
            refreshHandler.postDelayed(this, REFRESH_PERIOD_MS);
        }
    };

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        configureSystemBars();
        dashboard = new DashboardUi(this);
        if (savedInstanceState == null) {
            handleIntent(getIntent());
        } else {
            requestedConfig = savedInstanceState.getParcelable("vpnConfig");
            vpnConsentPending = savedInstanceState.getBoolean("vpnConsentPending");
            startRequestedByPhone = savedInstanceState.getBoolean("startRequestedByPhone");
        }
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        handleIntent(intent);
    }

    @Override
    protected void onSaveInstanceState(@NonNull Bundle state) {
        super.onSaveInstanceState(state);
        state.putParcelable("vpnConfig", requestedConfig);
        state.putBoolean("vpnConsentPending", vpnConsentPending);
        state.putBoolean("startRequestedByPhone", startRequestedByPhone);
    }

    private void handleIntent(Intent intent) {
        String action = intent.getAction();
        if (GnirehtetActivity.ACTION_GNIREHTET_START.equals(action)) {
            refreshHandler.removeCallbacks(restartRunnable);
            requestedConfig = configFromIntent(intent);
            prepareAndStart(false);
        } else if (GnirehtetActivity.ACTION_GNIREHTET_STOP.equals(action)) {
            stopLocally();
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

    private void configureSystemBars() {
        Window window = getWindow();
        WindowCompat.setDecorFitsSystemWindows(window, false);
        window.setStatusBarColor(Color.TRANSPARENT);
        window.setNavigationBarColor(Color.TRANSPARENT);
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

    @Override
    protected void onDestroy() {
        refreshHandler.removeCallbacks(restartRunnable);
        super.onDestroy();
    }

    void stopVpn() {
        SessionRequests.send(this, SessionRequests.STOP);
        stopLocally();
    }

    private void stopLocally() {
        refreshHandler.removeCallbacks(restartRunnable);
        vpnConsentPending = false;
        GnirehtetService.stop(this);
    }

    void restartVpn() {
        SessionRequests.send(this, SessionRequests.RESTART);
        stopLocally();
        refreshHandler.postDelayed(restartRunnable, RESTART_DELAY_MS);
    }

    void openBatteryOptimization() {
        Intent intent = new Intent(android.provider.Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS,
                Uri.parse("package:" + getPackageName()));
        try {
            startActivity(intent);
        } catch (RuntimeException error) {
            Log.w(TAG, "Could not open battery optimization settings: " + error);
        }
    }

    void requestStart() {
        prepareAndStart(true);
    }

    private void prepareAndStart(boolean fromPhone) {
        if (vpnConsentPending) {
            if (!fromPhone) {
                startRequestedByPhone = false;
            }
            return;
        }
        if (GnirehtetService.isRunning()) {
            return;
        }
        VpnConfiguration config = requestedConfig != null ? requestedConfig : new VpnConfiguration();
        Intent vpnIntent = VpnService.prepare(this);
        if (vpnIntent == null) {
            startVpn(config, fromPhone);
            return;
        }
        requestedConfig = config;
        vpnConsentPending = true;
        startRequestedByPhone = fromPhone;
        startActivityForResult(vpnIntent, VPN_REQUEST_CODE);
    }

    private void startVpn(VpnConfiguration config, boolean fromPhone) {
        if (fromPhone) {
            SessionRequests.send(this, SessionRequests.START);
        }
        GnirehtetService.start(this, config);
        requestNotificationPermissionIfNeeded();
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
        if (requestCode == VPN_REQUEST_CODE && vpnConsentPending) {
            vpnConsentPending = false;
            if (resultCode == RESULT_OK) {
                VpnConfiguration config = requestedConfig != null ? requestedConfig : new VpnConfiguration();
                startVpn(config, startRequestedByPhone);
            }
        }
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, @NonNull String[] permissions,
            @NonNull int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        if (requestCode == NOTIFICATION_PERMISSION_REQUEST_CODE) {
            Log.d(TAG, "Notification permission result delivered");
        }
    }

    void exportDiagnostics() {
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
}
