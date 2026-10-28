local id = ARGV[1]
local k = key(id)
if redis.call('EXISTS',k) ~= 0 then return redis.error_reply('duplicate queue job') end
redis.call('HSET',k,'id',id,'kind',ARGV[2],'payload_version',ARGV[3],'payload_json',ARGV[4],
    'status','pending','available_at',ARGV[5],'attempts','0','max_attempts',ARGV[6],'created_at',ARGV[7])
local m = ARGV[7] .. ':' .. id
redis.call('ZADD',all,0,m)
redis.call('ZADD',index('pending'),0,m)
redis.call('ZADD',due('pending'),0,ARGV[5] .. ':' .. id)
return 1
