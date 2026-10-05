"""Inspect official static font metadata using only Python's standard library."""
import argparse
import hashlib
import json
import struct
from pathlib import Path


def u16(data,offset):
    return struct.unpack_from('>H',data,offset)[0]


def names(data,offset):
    count=u16(data,offset+2)
    storage=offset+u16(data,offset+4)
    values={}
    for index in range(count):
        platform,encoding,language,identifier,length,start=struct.unpack_from('>6H',data,offset+6+12*index)
        if identifier not in (1,2,5,6):
            continue
        raw=data[storage+start:storage+start+length]
        value=raw.decode('utf-16-be' if platform in (0,3) else 'mac_roman')
        if identifier not in values or (platform==3 and language==0x409):
            values[identifier]=value
    return {label:values.get(identifier) for identifier,label in ((1,'family'),(2,'subfamily'),(5,'version'),(6,'postscriptName'))}


def coverage(data,offset):
    result=set()
    for index in range(u16(data,offset+2)):
        platform,encoding,start=struct.unpack_from('>2HI',data,offset+4+index*8)
        if platform not in (0,3):
            continue
        table=offset+start
        fmt=u16(data,table)
        if fmt==4:
            count=u16(data,table+6)//2
            end=table+14
            begins=end+2*count+2
            delta=begins+2*count
            ranges=delta+2*count
            for segment in range(count):
                lo=u16(data,begins+2*segment)
                hi=u16(data,end+2*segment)
                change=u16(data,delta+2*segment)
                pointer=u16(data,ranges+2*segment)
                for code in range(lo,min(hi,0xFFFE)+1):
                    glyph=(code+change)&0xFFFF if not pointer else u16(data,ranges+2*segment+pointer+2*(code-lo))
                    if pointer and glyph:
                        glyph=(glyph+change)&0xFFFF
                    if glyph:
                        result.add(code)
        elif fmt==12:
            groups=struct.unpack_from('>I',data,table+12)[0]
            for group in range(groups):
                lo,hi,glyph=struct.unpack_from('>3I',data,table+16+12*group)
                result.update(range(lo+(glyph==0),hi+1))
    return result


def inspect(path):
    data=path.read_bytes()
    if data[:4]!=b'\x00\x01\x00\x00':
        raise ValueError('Expected official TrueType SFNT.')
    tables={}
    for index in range(u16(data,4)):
        tag,checksum,offset,length=struct.unpack_from('>4s3I',data,12+index*16)
        if offset+length>len(data):
            raise ValueError('Font table escapes file.')
        tables[tag.decode('ascii')]=offset
    weight=u16(data,tables['OS/2']+4)
    selection=u16(data,tables['OS/2']+62)
    codes=coverage(data,tables['cmap'])
    samples={'latin':'Translate Settings 0123456789','vietnamese':'Tiếng Việt đăâêôơư Ắằễự','cjk':'繁體中文日本語한국어','thai':'ภาษาไทย'}
    return {'file':path.name,'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest(),**names(data,tables['name']),'weight':weight,'italic':bool(selection&1),'bold':bool(selection&32),'variable':('fvar' in tables),'unicodeGlyphs':len(codes),'sampleCoverage':{name:all(ord(character) in codes for character in text if not character.isspace()) for name,text in samples.items()}}


def main(args):
    package=Path(args.package).resolve()
    mappings={'Regular':(400,False),'Italic':(400,True),'Bold':(700,False),'BoldItalic':(700,True)}
    metadata=[]
    for style,(weight,italic) in mappings.items():
        record=inspect(package/'ttf'/f'Libron-{style}.ttf')
        if record['weight']!=weight or record['italic']!=italic or record['variable'] or record['family']!='Libron':
            raise ValueError(f'Unexpected actual static face metadata: {record}')
        webfont=(package/'woff2'/f'Libron-{style}.woff2').read_bytes()
        if webfont[:4]!=b'wOF2' or struct.unpack_from('>I',webfont,8)[0]!=len(webfont):
            raise ValueError('Invalid official WOFF2 container length or signature.')
        record['woff2Bytes']=len(webfont)
        record['woff2SHA256']=hashlib.sha256(webfont).hexdigest()
        metadata.append(record)
    result={'staticFourStyles':'PASS','fonts':metadata}
    evidence=Path(args.evidence)
    if evidence.exists():
        raise ValueError('Do not overwrite metadata evidence.')
    evidence.parent.mkdir(parents=True,exist_ok=True)
    evidence.write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps(result,ensure_ascii=True,indent=2))


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--package',required=True)
    parser.add_argument('--evidence',required=True)
    main(parser.parse_args())
