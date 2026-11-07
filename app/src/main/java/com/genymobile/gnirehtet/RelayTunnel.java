/*
 * Copyright (C) 2017 Genymobile
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

import android.net.LocalSocket;
import android.net.LocalSocketAddress;
import android.net.VpnService;
import android.util.Log;

import java.io.DataInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.SocketTimeoutException;

public final class RelayTunnel implements Tunnel {

    private static final String TAG = RelayTunnel.class.getSimpleName();

    private static final String LOCAL_ABSTRACT_NAME = "gnirehtet";
    private static final int GREETING_HOSTNAME_LENGTH = 256;
    private static final int GREETING_READ_TIMEOUT_MS = 1000;

    private final LocalSocket localSocket = new LocalSocket();

    private RelayTunnel() {
        // exposed through open() static method
    }

    @SuppressWarnings("unused")
    public static RelayTunnel open(VpnService vpnService) throws IOException {
        Log.d(TAG, "Opening a new relay tunnel...");
        // since we use a local socket, we don't need to protect the socket from the vpnService anymore
        // but this is an implementation detail, so keep the method signature
        return new RelayTunnel();
    }

    public void connect() throws IOException {
        localSocket.connect(new LocalSocketAddress(LOCAL_ABSTRACT_NAME));
        readClientId(localSocket.getInputStream());
        readGreeting(localSocket);
    }

    /**
     * The relay server is accessible through an "adb reverse" port redirection.
     * <p>
     * If the port redirection is enabled but the relay server is not started, then the call to
     * channel.connect() will succeed, but the first read() will return -1.
     * <p>
     * As a consequence, the connection state of the relay server would be invalid temporarily (we
     * would switch to CONNECTED state then switch back to DISCONNECTED).
     * <p>
     * To avoid this problem, we must actually read from the server, so that an error occurs
     * immediately if the relay server is not accessible.
     * <p>
     * Therefore, the relay server immediately sends the client id: consume it and log it.
     *
     * @param inputStream the input stream to receive data from the relay server
     * @throws IOException if an I/O error occurs
     */
    private static void readClientId(InputStream inputStream) throws IOException {
        Log.d(TAG, "Requesting client id");
        int clientId = new DataInputStream(inputStream).readInt();
        Log.d(TAG, "Connected to the relay server as #" + Binary.unsigned(clientId));
    }

    /**
     * After the client id, the Gnirehtet NG relay sends a fixed-size greeting (relay port and
     * host name). Legacy relays stop after the client id, so tolerate a missing greeting with a
     * short read timeout instead of blocking the tunnel forever.
     */
    private static void readGreeting(LocalSocket socket) throws IOException {
        try {
            socket.setSoTimeout(GREETING_READ_TIMEOUT_MS);
            DataInputStream input = new DataInputStream(socket.getInputStream());
            int relayPort = input.readUnsignedShort();
            byte[] hostname = new byte[GREETING_HOSTNAME_LENGTH];
            input.readFully(hostname);
            int length = 0;
            while (length < hostname.length && hostname[length] != 0) {
                length++;
            }
            SessionInfo.setGreeting(relayPort, new String(hostname, 0, length, "UTF-8"));
            Log.d(TAG, "Relay greeting: port " + relayPort);
        } catch (SocketTimeoutException timeout) {
            Log.w(TAG, "No relay greeting after the client id; assuming a legacy relay");
            SessionInfo.setGreeting(0, "");
        } finally {
            socket.setSoTimeout(0);
        }
    }

    @Override
    public void send(byte[] packet, int len) throws IOException {
        if (GnirehtetService.VERBOSE) {
            Log.v(TAG, "Sending packet: " + Binary.buildPacketString(packet, len));
        }
        localSocket.getOutputStream().write(packet, 0, len);
    }

    @Override
    public int receive(byte[] packet) throws IOException {
        int r = localSocket.getInputStream().read(packet);
        if (GnirehtetService.VERBOSE) {
            Log.v(TAG, "Receiving packet: " + Binary.buildPacketString(packet, r));
        }
        return r;
    }

    @Override
    public void close() {
        try {
            if (localSocket.getFileDescriptor() != null) {
                // close the streams to interrupt pending read and writes
                localSocket.shutdownInput();
                localSocket.shutdownOutput();
            }
            localSocket.close();
        } catch (IOException e) {
            // Cleanup must never crash the VPN process; the socket is unusable either way.
            Log.w(TAG, "Could not completely close relay tunnel", e);
        }
    }
}
