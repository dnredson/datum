import socket,struct,json,secrets,datetime,time
from pathlib import Path

def varint(n):
 out=bytearray()
 while n>127:out.append((n&127)|128);n>>=7
 out.append(n);return bytes(out)
def text(s):
 b=s.encode();return struct.pack('!H',len(b))+b
def packet(kind,payload):return bytes([kind])+varint(len(payload))+payload
def exact(sock,n):
 out=b''
 while len(out)<n:
  b=sock.recv(n-len(out))
  if not b:raise RuntimeError('MQTT connection closed')
  out+=b
 return out
def receive(sock):
 kind=exact(sock,1)[0];n=0;shift=0
 while True:
  b=exact(sock,1)[0];n|=(b&127)<<shift
  if b<128:break
  shift+=7
  if shift>21:raise ValueError('Invalid MQTT length')
 return kind,exact(sock,n)
def decode_proto(data):
 pos=0;fields={}
 def integer():
  nonlocal pos
  n=0;shift=0
  while True:
   b=data[pos];pos+=1;n|=(b&127)<<shift
   if b<128:return n
   shift+=7
   if shift>63:raise ValueError('Invalid protobuf varint')
 while pos<len(data):
  tag=integer();field=tag>>3;wire=tag&7
  if wire==0:value=integer()
  elif wire==2:
   size=integer();value=data[pos:pos+size];pos+=size
  elif wire in (1,5):
   size=8 if wire==1 else 4;value=data[pos:pos+size];pos+=size
  else:raise ValueError('Unsupported wire type')
  fields.setdefault(field,[]).append(value)
 return fields

def run():
 gateway='da7a'+secrets.token_hex(6)
 topic=f'eu868/gateway/{gateway}/event/stats'
 now=datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0)
 stats={'time':now.strftime('%Y-%m-%d %H:%M:%S GMT'),'rxnb':7,'rxok':7,'rxfw':7,'ackr':100.0,'dwnb':0,'txnb':0}
 token=secrets.token_bytes(2)
 frame=b'\x02'+token+b'\x00'+bytes.fromhex(gateway)+json.dumps({'stat':stats},separators=(',',':')).encode()
 with socket.create_connection(('192.168.10.103',1883),timeout=10) as mqtt, socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as udp:
  mqtt.settimeout(10);udp.settimeout(10)
  mqtt.sendall(packet(0x10,text('MQTT')+b'\x04\x02\x00\x1e'+text('datum-'+gateway)))
  kind,body=receive(mqtt);assert kind==0x20 and body==b'\x00\x00',(kind,body)
  mqtt.sendall(packet(0x82,b'\x00\x01'+text(topic)+b'\x00'))
  kind,body=receive(mqtt);assert kind==0x90 and body==b'\x00\x01\x00',(kind,body)
  started=time.monotonic();udp.sendto(frame,('192.168.10.109',1700))
  ack,peer=udp.recvfrom(4096)
  assert peer==('192.168.10.109',1700) and ack==b'\x02'+token+b'\x01',(peer,ack.hex())
  kind,body=receive(mqtt);assert kind>>4==3,kind
  size=int.from_bytes(body[:2],'big');actual=body[2:2+size].decode();payload=body[2+size:]
  assert actual==topic and not kind&1,(actual,kind)
  fields=decode_proto(payload)
  actual_gateway=fields[17][0].decode()
  received=fields.get(5,[0])[0];received_ok=fields.get(6,[0])[0]
  stamp=decode_proto(fields[2][0])[1][0]
  assert actual_gateway==gateway,(actual_gateway,gateway)
  assert received==7 and received_ok==7,(received,received_ok)
  assert stamp==int(now.timestamp()),(stamp,int(now.timestamp()))
  mqtt.sendall(b'\xe0\x00')
 report={'status':'passed','test':'Semtech PUSH_DATA stats to MQTT GatewayStats','sent_at_utc':now.isoformat(),'gateway_id':gateway,'topic':topic,'udp_destination':'192.168.10.109:1700','mqtt_broker':'192.168.10.103:1883','push_ack_verified':True,'mqtt_suback_verified':True,'retained':False,'gateway_id_verified':True,'timestamp_verified':True,'rx_packets_received':received,'rx_packets_received_ok':received_ok,'elapsed_ms':round((time.monotonic()-started)*1000,2),'protobuf_payload_hex':payload.hex(),'udp_packet_hex':frame.hex(),'scope':'Gateway statistics transport only; no LoRaWAN device uplink, ChirpStack server, SMA or storage validation'}
 out=Path('outputs');out.mkdir(exist_ok=True)
 (out/'mist-udp-mqtt-functional-test.json').write_text(json.dumps(report,indent=2)+'\n')
 print(json.dumps({k:v for k,v in report.items() if not k.endswith('_hex')},indent=2))
if __name__=='__main__':run()
