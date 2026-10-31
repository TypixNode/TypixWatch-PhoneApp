package com.typixnode.watch.platform

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.BluetoothStatusCodes
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.os.Build
import android.os.ParcelUuid
import java.util.ArrayDeque
import java.util.UUID
import uniffi.typix_core.WatchLink
import uniffi.typix_core.bleRxUuid
import uniffi.typix_core.bleServiceUuid
import uniffi.typix_core.bleTxUuid

data class FoundWatch(val device: BluetoothDevice, val name: String, val rssi: Int)

private val CCCD = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")

/** The caller holds the runtime Bluetooth permissions before anything here runs. */
@SuppressLint("MissingPermission")
class BleScanner(context: Context, private val found: (FoundWatch) -> Unit) {
    private val adapter = context.getSystemService(BluetoothManager::class.java)?.adapter

    private val callback = object : ScanCallback() {
        override fun onScanResult(type: Int, result: ScanResult) {
            val name = result.scanRecord?.deviceName ?: "W1A"
            found(FoundWatch(result.device, name, result.rssi))
        }
    }

    val available: Boolean get() = adapter?.isEnabled == true

    fun start() {
        val filter = ScanFilter.Builder()
            .setServiceUuid(ParcelUuid.fromString(bleServiceUuid()))
            .build()
        val settings = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
            .build()
        adapter?.bluetoothLeScanner?.startScan(listOf(filter), settings, callback)
    }

    fun stop() {
        adapter?.bluetoothLeScanner?.stopScan(callback)
    }
}

/**
 * One GATT connection as a byte pipe.  Android allows one outstanding GATT write, so
 * bytes queue here and the next piece leaves from `onCharacteristicWrite`.
 */
@SuppressLint("MissingPermission")
class BleLink(
    private val context: Context,
    private val device: BluetoothDevice,
    private val opened: (Boolean) -> Unit,
    private val received: (ByteArray) -> Unit,
    private val closed: () -> Unit,
) : WatchLink {
    private val lock = Any()
    private val pending = ArrayDeque<ByteArray>()
    private var writing = false
    private var pieceSize = 20
    private var open = false
    private var gatt: BluetoothGatt? = null
    private var rx: BluetoothGattCharacteristic? = null

    fun start() {
        gatt = device.connectGatt(context, false, callback, BluetoothDevice.TRANSPORT_LE)
    }

    private val callback = object : BluetoothGattCallback() {
        override fun onConnectionStateChange(g: BluetoothGatt, status: Int, state: Int) {
            if (state == BluetoothProfile.STATE_CONNECTED) {
                g.requestMtu(517)
            } else if (state == BluetoothProfile.STATE_DISCONNECTED) {
                g.close()
                if (open) closed() else opened(false)
                open = false
            }
        }

        override fun onMtuChanged(g: BluetoothGatt, mtu: Int, status: Int) {
            if (status == BluetoothGatt.GATT_SUCCESS) pieceSize = mtu - 3
            g.discoverServices()
        }

        override fun onServicesDiscovered(g: BluetoothGatt, status: Int) {
            val service = g.getService(UUID.fromString(bleServiceUuid()))
            val tx = service?.getCharacteristic(UUID.fromString(bleTxUuid()))
            rx = service?.getCharacteristic(UUID.fromString(bleRxUuid()))
            val descriptor = tx?.getDescriptor(CCCD)
            if (tx == null || rx == null || descriptor == null) {
                g.disconnect()
                return
            }
            g.setCharacteristicNotification(tx, true)
            writeDescriptor(g, descriptor)
        }

        override fun onDescriptorWrite(g: BluetoothGatt, d: BluetoothGattDescriptor, status: Int) {
            if (status != BluetoothGatt.GATT_SUCCESS) {
                g.disconnect()
                return
            }
            open = true
            opened(true)
        }

        override fun onCharacteristicChanged(
            g: BluetoothGatt,
            c: BluetoothGattCharacteristic,
            value: ByteArray,
        ) = received(value)

        @Deprecated("Below Android 13 the value only arrives here.")
        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
                @Suppress("DEPRECATION")
                received(c.value)
            }
        }

        override fun onCharacteristicWrite(
            g: BluetoothGatt,
            c: BluetoothGattCharacteristic,
            status: Int,
        ) {
            synchronized(lock) { writing = false }
            pump()
        }
    }

    @Suppress("DEPRECATION")
    private fun writeDescriptor(g: BluetoothGatt, d: BluetoothGattDescriptor) {
        val value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            g.writeDescriptor(d, value)
        } else {
            d.value = value
            g.writeDescriptor(d)
        }
    }

    override fun send(bytes: ByteArray) {
        synchronized(lock) {
            var off = 0
            while (off < bytes.size) {
                val end = minOf(off + pieceSize, bytes.size)
                pending.add(bytes.copyOfRange(off, end))
                off = end
            }
        }
        pump()
    }

    @Suppress("DEPRECATION")
    private fun pump() {
        val g = gatt ?: return
        val c = rx ?: return
        val piece = synchronized(lock) {
            if (writing) return
            pending.poll()?.also { writing = true }
        } ?: return
        val type = BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
        val ok = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            g.writeCharacteristic(c, piece, type) == BluetoothStatusCodes.SUCCESS
        } else {
            c.writeType = type
            c.value = piece
            g.writeCharacteristic(c)
        }
        if (!ok) g.disconnect()
    }

    override fun closeLink() {
        gatt?.disconnect()
    }
}
