import struct
from dataclasses import dataclass
from typing import List

import ms_toollib as ms


class CoreBaseVideo(ms.BaseVideo):
    mouse_state = 1
    game_board_state = 1
    x_y = (0, 0)

    def __new__(cls, board, cell_pixel_size):
        return ms.BaseVideo.__new__(cls, board, cell_pixel_size)

    def __init__(self, board, cell_pixel_size):
        super(CoreBaseVideo, self).__init__()

    @property
    def game_board(self):
        return self._game_board

    @game_board.setter
    def game_board(self, game_board):
        self._game_board = game_board

    class AlwaysZero:
        def __getitem__(self, key):
            class Inner:
                def __getitem__(self, inner_key):
                    return 0
            return Inner()
    game_board_poss = AlwaysZero()


@dataclass
class StatsRecord:
    game_state: int
    row: int
    column: int
    mine_num: int
    rtime_ms: int
    left: int
    right: int
    double: int
    rce: int
    lce: int
    dce: int
    bbbv: int
    bbbv_solved: int
    zini: int
    flag: int
    path: float
    start_time: int
    mode: int
    is_official: bool
    is_fair: bool
    op: int
    isl: int
    pluck: float
    short_md5: bytes = b""
    board_bytes: bytes = b""

    def encode(self) -> bytes:
        buf = bytearray()
        buf.extend(self.game_state.to_bytes(1, 'big'))
        buf.extend(self.row.to_bytes(1, 'big'))
        buf.extend(self.column.to_bytes(1, 'big'))
        buf.extend(self.mine_num.to_bytes(2, 'big'))
        buf.extend(self.rtime_ms.to_bytes(4, 'big'))
        buf.extend(self.left.to_bytes(4, 'big'))
        buf.extend(self.right.to_bytes(4, 'big'))
        buf.extend(self.double.to_bytes(4, 'big'))
        buf.extend(self.rce.to_bytes(4, 'big'))
        buf.extend(self.lce.to_bytes(4, 'big'))
        buf.extend(self.dce.to_bytes(4, 'big'))
        buf.extend(self.bbbv.to_bytes(2, 'big'))
        buf.extend(self.bbbv_solved.to_bytes(2, 'big'))
        buf.extend(self.zini.to_bytes(2, 'big'))
        buf.extend(self.flag.to_bytes(4, 'big'))
        buf.extend(struct.pack('!d', self.path))
        buf.extend(struct.pack('!d', self.pluck))
        buf.extend(self.start_time.to_bytes(8, 'big'))
        buf.extend(self.mode.to_bytes(1, 'big'))
        buf.append(1 if self.is_official else 0)
        buf.append(1 if self.is_fair else 0)
        buf.extend(self.op.to_bytes(2, 'big'))
        buf.extend(self.isl.to_bytes(2, 'big'))
        md5 = self.short_md5
        if len(md5) < 8:
            md5 = md5.ljust(8, b'\x00')
        buf.extend(md5[:8])
        buf.extend(len(self.board_bytes).to_bytes(2, 'big'))
        buf.extend(self.board_bytes)
        return bytes(buf)

    @classmethod
    def decode(cls, data: bytes) -> 'StatsRecord':
        ptr = 0
        game_state = int.from_bytes(data[ptr:ptr+1], 'big'); ptr +=1
        row = int.from_bytes(data[ptr:ptr+1], 'big'); ptr +=1
        column = int.from_bytes(data[ptr:ptr+1], 'big'); ptr +=1
        mine_num = int.from_bytes(data[ptr:ptr+2], 'big'); ptr +=2
        rtime_ms = int.from_bytes(data[ptr:ptr+4], 'big'); ptr +=4
        left = int.from_bytes(data[ptr:ptr+4], 'big'); ptr +=4
        right = int.from_bytes(data[ptr:ptr+4], 'big'); ptr +=4
        double = int.from_bytes(data[ptr:ptr+4], 'big'); ptr +=4
        rce = int.from_bytes(data[ptr:ptr+4], 'big'); ptr +=4
        lce = int.from_bytes(data[ptr:ptr+4], 'big'); ptr +=4
        dce = int.from_bytes(data[ptr:ptr+4], 'big'); ptr +=4
        bbbv = int.from_bytes(data[ptr:ptr+2], 'big'); ptr +=2
        bbbv_solved = int.from_bytes(data[ptr:ptr+2], 'big'); ptr +=2
        zini = int.from_bytes(data[ptr:ptr+2], 'big'); ptr +=2
        flag = int.from_bytes(data[ptr:ptr+4], 'big'); ptr +=4
        path = struct.unpack('!d', data[ptr:ptr+8])[0]; ptr +=8
        pluck = struct.unpack('!d', data[ptr:ptr+8])[0]; ptr +=8
        start_time = int.from_bytes(data[ptr:ptr+8], 'big'); ptr +=8
        mode = int.from_bytes(data[ptr:ptr+1], 'big'); ptr +=1
        is_official = data[ptr] == 1; ptr +=1
        is_fair = data[ptr] == 1; ptr +=1
        op = int.from_bytes(data[ptr:ptr+2], 'big'); ptr +=2
        isl = int.from_bytes(data[ptr:ptr+2], 'big'); ptr +=2
        short_md5 = data[ptr:ptr+8]; ptr += 8
        board_len = int.from_bytes(data[ptr:ptr+2], 'big'); ptr +=2
        board_bytes = data[ptr:ptr+board_len]; ptr += board_len
        return cls(
            game_state=game_state, row=row, column=column,
            mine_num=mine_num, rtime_ms=rtime_ms,
            left=left, right=right, double=double,
            rce=rce, lce=lce, dce=dce,
            bbbv=bbbv, bbbv_solved=bbbv_solved,
            zini=zini, flag=flag,
            path=path, start_time=start_time, mode=mode,
            is_official=is_official, is_fair=is_fair,
            op=op, isl=isl, pluck=pluck,
            short_md5=short_md5, board_bytes=board_bytes,
        )
