local id = ARGV[1]
local k = key(id)
check_type(k,'hash')
local s = redis.call('HGET',k,'status')
if not s then return 0 end
if s ~= 'failed' then return 1 end
member(k,id)
move(k,id,'failed','pending')
clear_lease(k)
redis.call('HDEL',k,'failed_at','completed_at','last_error_code','last_error_message')
redis.call('HSET',k,'attempts','0','available_at',ARGV[2])
redis.call('ZADD',due('pending'),0,ARGV[2] .. ':' .. id)
return 2
