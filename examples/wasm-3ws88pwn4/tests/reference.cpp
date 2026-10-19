// Build with /DREFERENCE_SOURCE=\".../s3hs88pwn4/sound.cpp\".
// The external source is read only; this harness freezes its observable output.
#include <cstring>
#include <fstream>
#include REFERENCE_SOURCE

int main(int argc, char** argv) {
    if (argc != 2) return 1;
    std::ofstream out(argv[1], std::ios::binary);
    for (int scenario=0;scenario<49;scenario++) {
        S3HS_sound chip;
        chip.initSound();
        auto write=[&](int r,int v){chip.ram_poke(chip.ram,0x400000+r,v);};
        // Keep silent channels' frequencies defined: the old zero-frequency
        // conversion has undefined float-to-int behavior.
        for(int ch=0;ch<8;ch++) {write(ch*64,2);write(ch*64+1,238);}
        for(int ch=0;ch<4;ch++) {write(0x200+ch*48,2);write(0x201+ch*48,238);}
        if(scenario<29) {
            for(int op=0;op<8;op++) {
                if(op) {write(op*2,16*(op+1));write(op*2+1,0);}
                write(16+op,80+op*9);
                write(32+op*4,0);write(33+op*4,2);write(34+op*4,190);write(35+op*4,2);
            }
            int wave=scenario<13 ? 0 : scenario-13;
            for(int i=0;i<4;i++) write(24+i,wave*17);
            write(28,scenario<13 ? scenario : 3);write(29,0xa7);write(30,1);write(31,128);
            for(int ch=0;ch<4;ch++) for(int i=0;i<32;i++) write(0x210+ch*48+i,(i*7+ch*31)%256);
            chip.resetGate(0);
        } else {
            int s=scenario-29;
            int mode=s<6?s:1;
            write(0x202,200);write(0x203,mode);write(0x207,0xf9);
            for(int i=0;i<32;i++) write(0x210+i,i*7);
            if(mode==0) {
                for(int i=0;i<256;i++) chip.ram_poke(chip.ram, i,(i*11)%256);
                write(0x210,0);write(0x211,0);write(0x212,0);
                write(0x213,0);write(0x214,0);write(0x215,255);
                write(0x216,0);write(0x217,0);write(0x218,32);
            }
            if(mode==5) {unsigned char data[4]={0,255,64,192};chip.putDMABuffer(0,data,4);}
            if(s>=6 && s<10) {write(0x204,s-6);write(0x205,90);write(0x206,45);}
            if(s>=10 && s<14) {write(0x2c1,1);write(0x2c5,24);write(0x2c6,16);write(0x2c7,32);}
            if(s>=14 && s<18) {write(0x2c0,1);write(0x2c2,80);write(0x2c3,150);write(0x2c4,32);}
            if(s==18) write(0x2d8,1);
            if(s==19) write(0x2d8,2);
        }
        auto frames=chip.AudioCallBack(512);
        for(int i=0;i<512;i++) for(int ch=0;ch<2;ch++) {
            short value=(short)frames[ch][12][i];
            out.put(value&255);out.put((unsigned short)value>>8);
        }
    }
    return !out;
}
