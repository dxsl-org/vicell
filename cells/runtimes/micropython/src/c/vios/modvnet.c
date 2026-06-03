/*
 * ViOS MicroPython `vnet` module.
 *
 * Exposes TCP socket IPC to Python scripts via:
 *   vnet.connect(ip_str, port_int)  -> cap_int | None
 *   vnet.send(cap_int, data)        -> bytes_sent
 *   vnet.recv(cap_int)              -> str | None
 *   vnet.close(cap_int)             -> None
 *   vnet.resolve(host_str)          -> ip_str | None
 *
 * Wire format mirrors bindings_net.rs in the Lua cell.
 * All IPC goes to the net service cell at endpoint 6.
 *
 * sys_recv returns the SENDER task ID, not a byte count — reply length is
 * recovered by zero-scanning a pre-zeroed buffer (done inside vios_net_recv).
 */

#include <stdint.h>
#include <stddef.h>
#include <string.h>

#include "py/obj.h"
#include "py/runtime.h"
#include "py/objstr.h"

/* ── ViOS syscall bridge (net_bridge.rs) ─────────────────────────────────── */
extern void     vios_net_send(size_t endpoint, const uint8_t *buf, size_t len);
extern intptr_t vios_net_recv(size_t from_id, uint8_t *buf, size_t buf_len);
extern void     vios_net_yield(void);

/* ── Constants ───────────────────────────────────────────────────────────── */
#define NET_ENDPOINT  6u
#define SOCKET_TCP    0x10u
#define CONNECT_OP    0x12u
#define SEND_OP       0x13u
#define RECV_OP       0x14u
#define CLOSE_OP      0x15u

#define MAX_SEND      480u
#define MAX_RECV      512u
#define POLL_RETRIES  500

/* ── Helpers ─────────────────────────────────────────────────────────────── */

static uint64_t read_le64(const uint8_t *b) {
    return  (uint64_t)b[0]        | ((uint64_t)b[1] << 8)  |
            ((uint64_t)b[2] << 16)| ((uint64_t)b[3] << 24) |
            ((uint64_t)b[4] << 32)| ((uint64_t)b[5] << 40) |
            ((uint64_t)b[6] << 48)| ((uint64_t)b[7] << 56);
}

static void write_le64(uint8_t *b, uint64_t v) {
    b[0]=(uint8_t)v;       b[1]=(uint8_t)(v>>8);
    b[2]=(uint8_t)(v>>16); b[3]=(uint8_t)(v>>24);
    b[4]=(uint8_t)(v>>32); b[5]=(uint8_t)(v>>40);
    b[6]=(uint8_t)(v>>48); b[7]=(uint8_t)(v>>56);
}

static void write_le16(uint8_t *b, uint16_t v) {
    b[0]=(uint8_t)v; b[1]=(uint8_t)(v>>8);
}

/* Parse dotted-decimal IPv4 "a.b.c.d" into out[4]. Returns 1 on success. */
static int parse_ipv4(const char *s, size_t slen, uint8_t out[4]) {
    unsigned int parts[4];
    int pi = 0;
    unsigned int cur = 0;
    int digits = 0;
    size_t i;
    for (i = 0; i <= slen && pi < 4; i++) {
        char c = (i < slen) ? s[i] : '.';
        if (c == '.') {
            if (digits == 0 || cur > 255) return 0;
            parts[pi++] = cur;
            cur = 0; digits = 0;
        } else if (c >= '0' && c <= '9') {
            cur = cur * 10u + (unsigned)(c - '0');
            digits++;
        } else {
            return 0;
        }
    }
    if (pi != 4) return 0;
    out[0]=(uint8_t)parts[0]; out[1]=(uint8_t)parts[1];
    out[2]=(uint8_t)parts[2]; out[3]=(uint8_t)parts[3];
    return 1;
}

/* Scan buf[0..n] for last non-zero byte. Returns byte count. */
static size_t zero_scan(const uint8_t *buf, size_t n) {
    while (n > 0 && buf[n-1] == 0) n--;
    return n;
}

/* ── vnet.connect(ip_str, port_int) -> cap_int | None ────────────────────── */

static mp_obj_t vnet_connect(mp_obj_t host_obj, mp_obj_t port_obj) {
    size_t hlen;
    const char *host = mp_obj_str_get_data(host_obj, &hlen);
    mp_int_t port = mp_obj_get_int(port_obj);

    uint8_t ip[4];
    if (!parse_ipv4(host, hlen, ip)) {
        return mp_const_none;
    }

    /* SOCKET_TCP → get cap handle */
    uint8_t sock_msg[9] = {SOCKET_TCP, 0,0,0,0,0,0,0,0};
    vios_net_send(NET_ENDPOINT, sock_msg, 9);
    uint8_t cap_buf[8];
    memset(cap_buf, 0, 8);
    if (vios_net_recv(0, cap_buf, 8) < 0) return mp_const_none;
    uint64_t cap = read_le64(cap_buf);
    if (cap == 0) return mp_const_none;

    /* CONNECT: [0x12][cap:8 LE][ip:4][port:2 LE] = 15 bytes */
    uint8_t conn[15];
    memset(conn, 0, 15);
    conn[0] = CONNECT_OP;
    write_le64(&conn[1], cap);
    conn[9]=ip[0]; conn[10]=ip[1]; conn[11]=ip[2]; conn[12]=ip[3];
    write_le16(&conn[13], (uint16_t)port);
    vios_net_send(NET_ENDPOINT, conn, 15);
    uint8_t ack[1];
    if (vios_net_recv(0, ack, 1) < 0 || ack[0] != 0x00) {
        /* close on failure so the cap is returned to the pool */
        uint8_t cl[9]; memset(cl, 0, 9); cl[0] = CLOSE_OP;
        write_le64(&cl[1], cap);
        vios_net_send(NET_ENDPOINT, cl, 9);
        uint8_t dummy[1];
        vios_net_recv(0, dummy, 1);
        return mp_const_none;
    }

    return mp_obj_new_int((mp_int_t)cap);
}
static MP_DEFINE_CONST_FUN_OBJ_2(vnet_connect_obj, vnet_connect);

/* ── vnet.send(cap_int, data) -> bytes_sent ──────────────────────────────── */

static mp_obj_t vnet_send(mp_obj_t cap_obj, mp_obj_t data_obj) {
    uint64_t cap = (uint64_t)mp_obj_get_int(cap_obj);
    size_t dlen;
    const char *data = mp_obj_str_get_data(data_obj, &dlen);
    if (dlen > MAX_SEND) dlen = MAX_SEND;

    size_t sent = 0;
    int i;
    for (i = 0; i < POLL_RETRIES && sent < dlen; i++) {
        size_t rem = dlen - sent;
        /* Build: [SEND_OP][cap:8 LE][data:rem] — stack-allocated */
        uint8_t msg[9 + MAX_SEND];
        msg[0] = SEND_OP;
        write_le64(&msg[1], cap);
        memcpy(&msg[9], data + sent, rem);
        vios_net_send(NET_ENDPOINT, msg, 9 + rem);
        uint8_t cnt[4];
        if (vios_net_recv(0, cnt, 4) < 0) break;
        uint32_t n = (uint32_t)cnt[0] | ((uint32_t)cnt[1]<<8) |
                     ((uint32_t)cnt[2]<<16) | ((uint32_t)cnt[3]<<24);
        sent += n;
        if (n == 0) vios_net_yield();
    }
    return mp_obj_new_int((mp_int_t)sent);
}
static MP_DEFINE_CONST_FUN_OBJ_2(vnet_send_obj, vnet_send);

/* ── vnet.recv(cap_int) -> str | None ────────────────────────────────────── */

static mp_obj_t vnet_recv(mp_obj_t cap_obj) {
    uint64_t cap = (uint64_t)mp_obj_get_int(cap_obj);

    /* RECV request: [RECV_OP][cap:8 LE][buf_len:4 LE] = 13 bytes */
    uint8_t req[13];
    req[0] = RECV_OP;
    write_le64(&req[1], cap);
    /* request up to MAX_RECV bytes */
    req[9]=(uint8_t)MAX_RECV; req[10]=(uint8_t)(MAX_RECV>>8);
    req[11]=0; req[12]=0;

    uint8_t reply[MAX_RECV];
    int i;
    for (i = 0; i < POLL_RETRIES; i++) {
        memset(reply, 0, MAX_RECV);
        vios_net_send(NET_ENDPOINT, req, 13);
        if (vios_net_recv(0, reply, MAX_RECV) >= 0 && reply[0] != 0) {
            /* find end: first NUL after start */
            size_t n = zero_scan(reply, MAX_RECV);
            if (n == 0) n = MAX_RECV;
            return mp_obj_new_str((const char *)reply, n);
        }
        vios_net_yield();
    }
    return mp_const_none;
}
static MP_DEFINE_CONST_FUN_OBJ_1(vnet_recv_obj, vnet_recv);

/* ── vnet.close(cap_int) -> None ─────────────────────────────────────────── */

static mp_obj_t vnet_close(mp_obj_t cap_obj) {
    uint64_t cap = (uint64_t)mp_obj_get_int(cap_obj);
    uint8_t msg[9];
    msg[0] = CLOSE_OP;
    write_le64(&msg[1], cap);
    vios_net_send(NET_ENDPOINT, msg, 9);
    uint8_t dummy[1];
    vios_net_recv(0, dummy, 1);
    return mp_const_none;
}
static MP_DEFINE_CONST_FUN_OBJ_1(vnet_close_obj, vnet_close);

/* ── vnet.resolve(host_str) -> ip_str | None ─────────────────────────────── */
/*
 * Resolution order:
 *   1. Static SLIRP aliases
 *   2. IPv4 literal pass-through
 * DNS is not implemented in the C module (use Lua vnet.resolve for DNS).
 */

static mp_obj_t vnet_resolve(mp_obj_t host_obj) {
    size_t hlen;
    const char *host = mp_obj_str_get_data(host_obj, &hlen);

    /* 1. Static table */
    static const struct { const char *name; uint8_t ip[4]; } STATICS[] = {
        { "gateway", {10,0,2,2} }, { "host", {10,0,2,2} },
        { "dns",     {10,0,2,3} }, { "localhost", {127,0,0,1} },
    };
    int si;
    for (si = 0; si < (int)(sizeof(STATICS)/sizeof(STATICS[0])); si++) {
        if (strlen(STATICS[si].name) == hlen &&
            memcmp(STATICS[si].name, host, hlen) == 0) {
            const uint8_t *a = STATICS[si].ip;
            char buf[16];
            int n = 0, oi;
            for (oi = 0; oi < 4; oi++) {
                if (oi > 0) buf[n++] = '.';
                unsigned v = a[oi];
                if (v >= 100) { buf[n++]=(char)('0'+v/100); v%=100; buf[n++]=(char)('0'+v/10); }
                else if (v >= 10) { buf[n++]=(char)('0'+v/10); }
                buf[n++]=(char)('0'+v%10);
            }
            return mp_obj_new_str(buf, (size_t)n);
        }
    }

    /* 2. IPv4 literal */
    uint8_t ip[4];
    if (parse_ipv4(host, hlen, ip)) {
        return mp_obj_new_str(host, hlen);
    }

    return mp_const_none;
}
static MP_DEFINE_CONST_FUN_OBJ_1(vnet_resolve_obj, vnet_resolve);

/* ── Module table ────────────────────────────────────────────────────────── */

static const mp_rom_map_elem_t vnet_module_globals_table[] = {
    { MP_ROM_QSTR(MP_QSTR___name__), MP_ROM_QSTR(MP_QSTR_vnet) },
    { MP_ROM_QSTR(MP_QSTR_connect),  MP_ROM_PTR(&vnet_connect_obj) },
    { MP_ROM_QSTR(MP_QSTR_send),     MP_ROM_PTR(&vnet_send_obj) },
    { MP_ROM_QSTR(MP_QSTR_recv),     MP_ROM_PTR(&vnet_recv_obj) },
    { MP_ROM_QSTR(MP_QSTR_close),    MP_ROM_PTR(&vnet_close_obj) },
    { MP_ROM_QSTR(MP_QSTR_resolve),  MP_ROM_PTR(&vnet_resolve_obj) },
};
static MP_DEFINE_CONST_DICT(vnet_module_globals, vnet_module_globals_table);

const mp_obj_module_t mp_module_vnet = {
    .base    = { &mp_type_module },
    .globals = (mp_obj_dict_t *)&vnet_module_globals,
};

/* Register as a built-in module named `vnet`. */
MP_REGISTER_MODULE(MP_QSTR_vnet, mp_module_vnet);
