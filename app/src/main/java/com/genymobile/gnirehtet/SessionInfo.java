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

/**
 * Session metadata shared between the VPN service and the dashboard: relay greeting
 * (relay port and PC host name) and the active VPN configuration. The UI polls these
 * volatile fields, keeping the UI process decoupled from the network service lifecycle.
 */
public final class SessionInfo {

    private static volatile int relayPort;
    private static volatile String pcName = "";
    private static volatile String dnsServers = "";
    private static volatile String routes = "";

    private SessionInfo() {
        // utility class
    }

    public static void setGreeting(int port, String name) {
        relayPort = port;
        pcName = name;
    }

    public static int getRelayPort() {
        return relayPort;
    }

    public static String getPcName() {
        return pcName;
    }

    public static void setConfiguration(String dns, String routesValue) {
        dnsServers = dns;
        routes = routesValue;
    }

    public static String getDnsServers() {
        return dnsServers;
    }

    public static String getRoutes() {
        return routes;
    }

    public static void reset() {
        relayPort = 0;
        pcName = "";
        dnsServers = "";
        routes = "";
    }
}
