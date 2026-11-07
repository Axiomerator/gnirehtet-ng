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

import android.content.Context;
import android.content.Intent;
import android.net.Network;
import android.net.VpnService;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.os.Message;
import android.os.ParcelFileDescriptor;
import android.system.OsConstants;
import android.util.Log;

import java.io.IOException;
import java.net.InetAddress;
import java.util.concurrent.atomic.AtomicLong;

public class GnirehtetService extends VpnService {

    public static final boolean VERBOSE = false;

    private static final String ACTION_START_VPN = BuildConfig.APPLICATION_ID + ".START_VPN";
    private static final String ACTION_STOP_VPN = BuildConfig.APPLICATION_ID + ".STOP_VPN";
    private static final String EXTRA_VPN_CONFIGURATION = "vpnConfiguration";

    private static final String TAG = GnirehtetService.class.getSimpleName();

    private static final InetAddress VPN_ADDRESS = Net.toInetAddress(new byte[] {10, 0, 0, 2});
    // magic value: higher (like 0x8000 or 0xffff) or lower (like 1500) values show poorer performances
    private static final int MTU = 0x4000;
    private static final int MSG_RELAY_TUNNEL_DISCONNECT_TIMEOUT = 2;
    private static final long RELAY_DISCONNECT_SHUTDOWN_DELAY_MS = 5000;

    // Dashboard state, decoupled from the UI process: the activity polls these volatile
    // fields, so the VPN keeps running when the UI is destroyed.
    private static volatile boolean vpnRunning;
    private static volatile boolean tunnelConnected;
    private static volatile long startedAtMs;
    private static final AtomicLong DOWN_BYTES = new AtomicLong();
    private static final AtomicLong UP_BYTES = new AtomicLong();

    private final Notifier notifier = new Notifier(this);
    private final Handler handler = new RelayTunnelConnectionStateHandler(this);

    private ParcelFileDescriptor vpnInterface;
    private Forwarder forwarder;
    private boolean closing;

    public static boolean isRunning() {
        return vpnRunning;
    }

    public static boolean isTunnelConnected() {
        return tunnelConnected;
    }

    public static long getStartedAtMs() {
        return startedAtMs;
    }

    public static long getDownBytes() {
        return DOWN_BYTES.get();
    }

    public static long getUpBytes() {
        return UP_BYTES.get();
    }

    public static void start(Context context, VpnConfiguration config) {
        Intent intent = new Intent(context, GnirehtetService.class);
        intent.setAction(ACTION_START_VPN);
        intent.putExtra(GnirehtetService.EXTRA_VPN_CONFIGURATION, config);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            context.startForegroundService(intent);
        } else {
            context.startService(intent);
        }
    }

    public static void stop(Context context) {
        Intent intent = new Intent(context, GnirehtetService.class).setAction(ACTION_STOP_VPN);
        context.startService(intent);
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && ACTION_STOP_VPN.equals(intent.getAction())) {
            Log.d(TAG, "Received STOP request");
            close();
            return START_NOT_STICKY;
        }

        if (intent == null || !ACTION_START_VPN.equals(intent.getAction())) {
            Log.w(TAG, "Ignoring empty or unknown start request");
            stopSelf(startId);
            return START_NOT_STICKY;
        }

        if (isVpnInterfaceRunning()) {
            Log.d(TAG, "VPN already running, ignoring START request");
            // Redeliver on process death so the system restores a session that a
            // background manager killed while the START request was still in effect.
            return START_REDELIVER_INTENT;
        }
        notifier.start();
        VpnConfiguration config = intent.getParcelableExtra(EXTRA_VPN_CONFIGURATION);
        if (config == null) {
            config = new VpnConfiguration();
        }

        if (setupVpn(config)) {
            vpnRunning = true;
            tunnelConnected = false;
            startedAtMs = System.currentTimeMillis();
            DOWN_BYTES.set(0);
            UP_BYTES.set(0);
            SessionInfo.setConfiguration(joinDnsServers(config), joinRoutes(config));
            startForwarding();
        } else {
            close();
        }
        // If a background manager kills this process mid-session, the system restarts
        // the service with the START intent and re-establishes the VPN on its own.
        return START_REDELIVER_INTENT;
    }

    private static String joinDnsServers(VpnConfiguration config) {
        StringBuilder builder = new StringBuilder();
        for (InetAddress address : config.getDnsServers()) {
            if (builder.length() != 0) {
                builder.append(", ");
            }
            builder.append(address.getHostAddress());
        }
        return builder.toString();
    }

    private static String joinRoutes(VpnConfiguration config) {
        StringBuilder builder = new StringBuilder();
        for (CIDR cidr : config.getRoutes()) {
            if (builder.length() != 0) {
                builder.append(", ");
            }
            builder.append(cidr.toString());
        }
        return builder.toString();
    }

    private synchronized boolean isVpnInterfaceRunning() {
        return vpnInterface != null && !closing;
    }

    @SuppressWarnings("checkstyle:MagicNumber")
    private boolean setupVpn(VpnConfiguration config) {
        Builder builder = new Builder()
                .addAddress(VPN_ADDRESS, 32)
                // The relay only transports IPv4; let IPv6 bypass this VPN instead of black-holing it.
                .allowFamily(OsConstants.AF_INET6)
                .setSession(getString(R.string.app_name))
                // FileChannel is not selectable, therefore use synchronous I/O instead of polling.
                .setBlocking(true)
                .setMtu(MTU);

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            builder.setMetered(false);
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            // Do not accidentally advertise this VPN as its own upstream network.
            builder.setUnderlyingNetworks(new Network[0]);
        }

        CIDR[] routes = config.getRoutes();
        if (routes.length == 0) {
            // no routes defined, redirect the whole network traffic
            builder.addRoute("0.0.0.0", 0);
        } else {
            for (CIDR route : routes) {
                builder.addRoute(route.getAddress(), route.getPrefixLength());
            }
        }

        InetAddress[] dnsServers = config.getDnsServers();
        if (dnsServers.length == 0) {
            // no DNS server defined, use Google DNS
            builder.addDnsServer("8.8.8.8");
        } else {
            for (InetAddress dnsServer : dnsServers) {
                builder.addDnsServer(dnsServer);
            }
        }

        try {
            vpnInterface = builder.establish();
        } catch (IllegalArgumentException | IllegalStateException | SecurityException e) {
            Log.e(TAG, "Could not establish VPN interface", e);
            return false;
        }
        if (vpnInterface == null) {
            Log.w(TAG, "VPN starting failed or authorization was revoked");
            return false;
        }
        return true;
    }

    private void startForwarding() {
        ParcelFileDescriptor descriptor = vpnInterface;
        if (descriptor == null) {
            close();
            return;
        }
        forwarder = new Forwarder(this, descriptor.getFileDescriptor(), new RelayTunnelListener(handler),
                DOWN_BYTES, UP_BYTES);
        forwarder.forward();
    }

    @Override
    public void onRevoke() {
        Log.i(TAG, "VPN authorization revoked");
        if (isVpnInterfaceRunning()) {
            SessionRequests.send(this, SessionRequests.STOP);
        }
        close();
        super.onRevoke();
    }

    @Override
    public void onDestroy() {
        close();
        super.onDestroy();
    }

    /**
     * ADB-observable state for remote tooling: read with
     * {@code adb shell dumpsys activity services org.gnirehtetng.client}.
     * <p>
     * Control remains available through the START/STOP activity actions used by the PC
     * controller.
     */
    @Override
    protected void dump(java.io.FileDescriptor fd, java.io.PrintWriter writer, String[] args) {
        writer.println("GNIREHTET_NG_SERVICE");
        writer.println("running=" + vpnRunning);
        writer.println("tunnel_connected=" + tunnelConnected);
        writer.println("started_at_ms=" + startedAtMs);
        writer.println("down_bytes=" + DOWN_BYTES.get());
        writer.println("up_bytes=" + UP_BYTES.get());
        writer.println("auto_reconnect=" + Settings.isAutoReconnectEnabled(this));
        writer.println("pc_name=" + SessionInfo.getPcName());
        writer.println("relay_port=" + SessionInfo.getRelayPort());
        writer.println("dns_servers=" + SessionInfo.getDnsServers());
        writer.println("routes=" + SessionInfo.getRoutes());
    }

    private synchronized void close() {
        if (closing) {
            return;
        }
        closing = true;

        Forwarder localForwarder = forwarder;
        forwarder = null;
        ParcelFileDescriptor localInterface = vpnInterface;
        vpnInterface = null;

        if (localForwarder != null) {
            localForwarder.stop();
        }
        if (localInterface != null) {
            try {
                localInterface.close();
            } catch (IOException e) {
                Log.w(TAG, "Cannot close VPN file descriptor", e);
            }
        }

        handler.removeMessages(MSG_RELAY_TUNNEL_DISCONNECT_TIMEOUT);
        notifier.stop();
        stopSelf();
        closing = false;
        vpnRunning = false;
        tunnelConnected = false;
        SessionInfo.reset();
    }


    private static final class RelayTunnelConnectionStateHandler extends Handler {

        private final GnirehtetService vpnService;

        private RelayTunnelConnectionStateHandler(GnirehtetService vpnService) {
            super(Looper.getMainLooper());
            this.vpnService = vpnService;
        }

        @Override
        public void handleMessage(Message message) {
            if (!vpnService.isVpnInterfaceRunning()) {
                // if the VPN is not running anymore, ignore obsolete events
                return;
            }
            switch (message.what) {
                case RelayTunnelListener.MSG_RELAY_TUNNEL_CONNECTED:
                    Log.d(TAG, "Relay tunnel connected");
                    tunnelConnected = true;
                    removeMessages(MSG_RELAY_TUNNEL_DISCONNECT_TIMEOUT);
                    vpnService.notifier.setFailure(false);
                    break;
                case RelayTunnelListener.MSG_RELAY_TUNNEL_DISCONNECTED:
                    Log.d(TAG, "Relay tunnel disconnected");
                    tunnelConnected = false;
                    vpnService.notifier.setFailure(true);
                    removeMessages(MSG_RELAY_TUNNEL_DISCONNECT_TIMEOUT);
                    sendEmptyMessageDelayed(MSG_RELAY_TUNNEL_DISCONNECT_TIMEOUT,
                            RELAY_DISCONNECT_SHUTDOWN_DELAY_MS);
                    break;
                case MSG_RELAY_TUNNEL_DISCONNECT_TIMEOUT:
                    if (Settings.isAutoReconnectEnabled(vpnService)) {
                        Log.i(TAG, "Relay lost but auto-reconnect is enabled; keeping the VPN alive");
                    } else {
                        Log.i(TAG, "Relay disconnected, stopping VPN");
                        vpnService.close();
                    }
                    break;
                default:
                    break;
            }
        }
    }
}
