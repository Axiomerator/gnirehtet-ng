package com.genymobile.gnirehtet;

import android.Manifest;
import android.app.Activity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.net.VpnService;
import android.os.Build;
import android.os.Bundle;
import android.util.Log;

/**
 * This (invisible) activity receives the {@link #ACTION_GNIREHTET_START START} and
 * {@link #ACTION_GNIREHTET_STOP} actions from the command line.
 * <p>
 * Recent versions of Android refuse to directly start a {@link android.app.Service Service} or a
 * {@link android.content.BroadcastReceiver BroadcastReceiver}, so actions are always managed by
 * this activity.
 */
public class GnirehtetActivity extends Activity {

    private static final String TAG = GnirehtetActivity.class.getSimpleName();

    public static final String ACTION_GNIREHTET_START = BuildConfig.APPLICATION_ID + ".START";
    public static final String ACTION_GNIREHTET_STOP = BuildConfig.APPLICATION_ID + ".STOP";

    public static final String EXTRA_DNS_SERVERS = "dnsServers";
    public static final String EXTRA_ROUTES = "routes";
    /**
     * When set, the activity restores the VPN without opening the dashboard and without
     * requesting any dialog, so watchdog recoveries stay completely invisible.
     */
    public static final String EXTRA_SILENT = "silent";

    private static final int VPN_REQUEST_CODE = 0;
    private static final int NOTIFICATION_PERMISSION_REQUEST_CODE = 1;

    private VpnConfiguration requestedConfig;
    private boolean silent;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        handleIntent(getIntent());
    }

    private void handleIntent(Intent intent) {
        silent = intent.getBooleanExtra(EXTRA_SILENT, false);
        String action = intent.getAction();
        Log.d(TAG, "Received request " + action + (silent ? " (silent)" : ""));
        boolean finish = true;
        if (ACTION_GNIREHTET_START.equals(action)) {
            VpnConfiguration config = createConfig(intent);
            finish = startGnirehtet(config);
        } else if (ACTION_GNIREHTET_STOP.equals(action)) {
            stopGnirehtet();
        }

        if (finish) {
            finish();
        }
    }

    private static VpnConfiguration createConfig(Intent intent) {
        String[] dnsServers = intent.getStringArrayExtra(EXTRA_DNS_SERVERS);
        if (dnsServers == null) {
            dnsServers = new String[0];
        }
        String[] routes = intent.getStringArrayExtra(EXTRA_ROUTES);
        if (routes == null) {
            routes = new String[0];
        }
        return new VpnConfiguration(Net.toInetAddresses(dnsServers), Net.toCIDRs(routes));
    }

    private boolean startGnirehtet(VpnConfiguration config) {
        Intent vpnIntent = VpnService.prepare(this);
        if (vpnIntent == null) {
            Log.d(TAG, "VPN was already authorized");
            // start the service immediately; the notification permission is requested
            // afterwards and must never block the connection
            GnirehtetService.start(this, config);
            if (!silent) {
                openDashboard();
                return !requestNotificationPermissionIfNeeded();
            }
            return true;
        }

        Log.w(TAG, "VPN requires the authorization from the user, requesting...");
        requestAuthorization(vpnIntent, config);
        return false; // do not finish now
    }

    /**
     * Request the notification permission at most once (persisted). Returns {@code true} if a
     * system dialog is now pending (the activity must stay alive until the result arrives).
     */
    private boolean requestNotificationPermissionIfNeeded() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            return false;
        }
        if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                == PackageManager.PERMISSION_GRANTED) {
            return false;
        }
        if (Settings.isNotificationPermissionRequested(this)) {
            return false;
        }
        Settings.setNotificationPermissionRequested(this, true);
        requestPermissions(new String[] {Manifest.permission.POST_NOTIFICATIONS},
                NOTIFICATION_PERMISSION_REQUEST_CODE);
        return true;
    }

    private void openDashboard() {
        Intent intent = new Intent(this, MainActivity.class);
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        startActivity(intent);
    }

    private void stopGnirehtet() {
        GnirehtetService.stop(this);
    }

    private void requestAuthorization(Intent vpnIntent, VpnConfiguration config) {
        this.requestedConfig = config;
        startActivityForResult(vpnIntent, VPN_REQUEST_CODE);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == VPN_REQUEST_CODE && resultCode == RESULT_OK) {
            VpnConfiguration config = requestedConfig != null ? requestedConfig : createConfig(getIntent());
            GnirehtetService.start(this, config);
            if (!silent) {
                openDashboard();
                if (requestNotificationPermissionIfNeeded()) {
                    requestedConfig = null;
                    return; // wait for the permission result before finishing
                }
            }
        }
        requestedConfig = null;
        finish();
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        // the VPN service already started; the notification permission only affects cosmetics
        if (requestCode == NOTIFICATION_PERMISSION_REQUEST_CODE) {
            finish();
        }
    }
}
